//! Plugin operations and a small, static-configuration lifecycle above log-proto transport.
use anyhow::{anyhow, bail, Context as _, Result};
use log_proto::{
    ClientConnection, ClientReader, ClientWriter, Fault, Hello, Record, Request, ServerMessage,
    TransportKind, EVENT_QUEUE, PROTOCOL,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    sync::{mpsc, oneshot, watch, Mutex, Semaphore},
    task::AbortHandle,
};
mod lifecycle;
pub use lifecycle::{bounded, connect, finish, stopped, termination, Context};
const TIMEOUT: Duration = Duration::from_secs(30);
#[derive(Clone, Debug)]
pub enum Event {
    Record(Record),
    Gap {
        stream: String,
        epoch: String,
        from: u64,
        to: u64,
        reason: String,
    },
    Disconnected {
        stream: String,
        reason: String,
    },
}
#[derive(Clone, Debug)]
pub struct Control {
    pub call_id: u64,
    pub method: String,
    pub args: Value,
}
/// TCP admission and UDP local transmission are deliberately distinct outcomes.
#[derive(Clone, Debug)]
pub enum PublishOutcome {
    Accepted(Box<Record>),
    LocalSent,
}
type Pending = Arc<std::sync::Mutex<BTreeMap<u64, oneshot::Sender<Result<Value, Fault>>>>>;
struct PendingGuard {
    id: u64,
    pending: Pending,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.lock().unwrap().remove(&self.id);
    }
}
struct ConnectionWriter(ClientWriter);
impl std::ops::Deref for ConnectionWriter {
    type Target = ClientWriter;
    fn deref(&self) -> &ClientWriter {
        &self.0
    }
}
impl std::ops::DerefMut for ConnectionWriter {
    fn deref_mut(&mut self) -> &mut ClientWriter {
        &mut self.0
    }
}
impl Drop for ConnectionWriter {
    fn drop(&mut self) {
        self.0.disconnect();
    }
}
struct Outbound {
    request: Request,
    sent: oneshot::Sender<Result<(), String>>,
}
struct Inner {
    address: String,
    plugin: String,
    token: String,
    config: Value,
    kind: TransportKind,
    own_stream: Option<String>,
    read_streams: Vec<String>,
    sequence: AtomicU64,
    outbound: mpsc::Sender<Outbound>,
    priority: mpsc::Sender<Outbound>,
    slots: Semaphore,
    control_slots: Semaphore,
    pending: Pending,
    events: mpsc::Sender<Event>,
    aliases: Mutex<BTreeMap<String, String>>,
    subscriptions: Mutex<BTreeMap<String, AbortHandle>>,
    reader: AbortHandle,
    writer: AbortHandle,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
        for handle in self.subscriptions.get_mut().values() {
            handle.abort();
        }
    }
}
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}
async fn socket(
    address: &str,
    kind: TransportKind,
    plugin: &str,
    token: &str,
    events: bool,
) -> Result<(ClientReader, ConnectionWriter, Value)> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (mut reader, writer) = ClientConnection::connect(
            address,
            kind,
            &Hello {
                protocol: PROTOCOL.into(),
                plugin: plugin.into(),
                token: token.into(),
                events,
            },
        )
        .await?;
        match reader.receive().await? {
            Some(ServerMessage::Response {
                id: 0,
                result,
                error: None,
            }) if result["protocol"] == PROTOCOL => Ok((reader, ConnectionWriter(writer), result)),
            Some(ServerMessage::Response {
                error: Some(error), ..
            }) => Err(error.into()),
            _ => bail!("invalid Core welcome"),
        }
    })
    .await
    .context("Core registration timed out")?
}
pub async fn connect_env() -> Result<(Client, mpsc::Receiver<Event>, mpsc::Receiver<Control>)> {
    let address = std::env::var("LOG_PRINT_CORE")
        .context("LOG_PRINT_CORE missing; launch plugin via log-print")?;
    let plugin = std::env::var("LOG_PRINT_PLUGIN")?;
    let token = std::env::var("LOG_PRINT_TOKEN")?;
    let config =
        serde_json::from_str(&std::env::var("LOG_PRINT_CONFIG").unwrap_or_else(|_| "{}".into()))?;
    let kind = match std::env::var("LOG_PRINT_TRANSPORT").as_deref() {
        Ok("udp") => TransportKind::Udp,
        Ok("tcp") | Err(_) => TransportKind::Tcp,
        Ok(other) => bail!("unknown transport {other}"),
    };
    Client::connect_with_transport(&address, &plugin, &token, config, kind).await
}
impl Client {
    pub async fn connect(
        address: &str,
        plugin: &str,
        token: &str,
    ) -> Result<(Self, mpsc::Receiver<Event>, mpsc::Receiver<Control>)> {
        Self::connect_with_config(address, plugin, token, json!({})).await
    }
    pub async fn connect_with_config(
        address: &str,
        plugin: &str,
        token: &str,
        config: Value,
    ) -> Result<(Self, mpsc::Receiver<Event>, mpsc::Receiver<Control>)> {
        Self::connect_with_transport(address, plugin, token, config, TransportKind::Tcp).await
    }
    pub async fn connect_with_transport(
        address: &str,
        plugin: &str,
        token: &str,
        config: Value,
        kind: TransportKind,
    ) -> Result<(Self, mpsc::Receiver<Event>, mpsc::Receiver<Control>)> {
        let (mut reader, mut writer, welcome) = socket(address, kind, plugin, token, false).await?;
        let (outbound, mut outgoing) = mpsc::channel::<Outbound>(32);
        let (priority, mut priority_rx) = mpsc::channel::<Outbound>(8);
        let (events, event_rx) = mpsc::channel(EVENT_QUEUE);
        let (controls, control_rx) = mpsc::channel(32);
        let pending: Pending = Arc::default();
        let (closed, mut close_rx) = watch::channel(false);
        let writer_closed = closed.clone();
        let writer_task = tokio::spawn(async move {
            loop {
                let item = tokio::select! { biased; _=close_rx.changed()=>break,Some(item)=priority_rx.recv()=>item,Some(item)=outgoing.recv()=>item,else=>break };
                // This task owns a complete request. Cancelling its caller cannot truncate a TCP frame.
                let result = tokio::time::timeout(TIMEOUT, writer.send(&item.request)).await;
                let outcome = match result {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err("transmission timed out; outcome unknown".into()),
                };
                let failed = outcome.is_err();
                let _ = item.sent.send(outcome);
                if failed {
                    writer_closed.send_replace(true);
                    break;
                }
            }
        });
        let reader_pending = pending.clone();
        let reader_events = events.clone();
        let mut reader_closed = closed.subscribe();
        let reader_task = tokio::spawn(async move {
            let reason = loop {
                let next = tokio::select! {_=reader_closed.changed()=>break "Core writer closed".to_string(),next=reader.receive()=>next};
                match next {
                    Ok(Some(ServerMessage::Response { id, result, error })) => {
                        if let Some(sender) = reader_pending.lock().unwrap().remove(&id) {
                            let _ = sender.send(match error {
                                Some(e) => Err(e),
                                None => Ok(result),
                            });
                        }
                    }
                    Ok(Some(ServerMessage::Control {
                        call_id,
                        method,
                        args,
                    })) => {
                        if controls
                            .try_send(Control {
                                call_id,
                                method,
                                args,
                            })
                            .is_err()
                        {
                            break "plugin control queue exhausted".into();
                        }
                    }
                    Ok(Some(_)) => break "unexpected event on operation connection".into(),
                    Ok(None) => break "Core connection closed".into(),
                    Err(e) => break e.to_string(),
                }
            };
            closed.send_replace(true);
            for (_, sender) in std::mem::take(&mut *reader_pending.lock().unwrap()) {
                let _ = sender.send(Err(Fault {
                    code: "connection_lost".into(),
                    message: format!("{reason}; unconfirmed outcomes unknown"),
                }));
            }
            let _ = reader_events.try_send(Event::Disconnected {
                stream: "*".into(),
                reason,
            });
        });
        let own_stream = welcome["stream"]["id"].as_str().map(str::to_owned);
        let read_streams = serde_json::from_value(welcome["reads"].clone()).unwrap_or_default();
        let mut aliases = BTreeMap::new();
        if let Some(id) = &own_stream {
            aliases.insert(id.clone(), id.clone());
            if let Some(alias) = welcome["stream"]["alias"].as_str() {
                aliases.insert(alias.into(), id.clone());
            }
        }
        Ok((
            Self {
                inner: Arc::new(Inner {
                    address: address.into(),
                    plugin: plugin.into(),
                    token: token.into(),
                    config,
                    kind,
                    own_stream,
                    read_streams,
                    sequence: AtomicU64::new(1),
                    outbound,
                    priority,
                    slots: Semaphore::new(32),
                    control_slots: Semaphore::new(8),
                    pending,
                    events,
                    aliases: Mutex::new(aliases),
                    subscriptions: Mutex::new(BTreeMap::new()),
                    reader: reader_task.abort_handle(),
                    writer: writer_task.abort_handle(),
                }),
            },
            event_rx,
            control_rx,
        ))
    }
    pub fn config(&self) -> &Value {
        &self.inner.config
    }
    pub fn stream_id(&self) -> Option<&str> {
        self.inner.own_stream.as_deref()
    }
    pub fn input_stream(&self) -> Option<&str> {
        self.stream_id()
    }
    pub fn own_stream(&self) -> Option<&str> {
        self.stream_id()
    }
    pub fn read_streams(&self) -> &[String] {
        &self.inner.read_streams
    }
    pub fn transport(&self) -> TransportKind {
        self.inner.kind
    }
    async fn send(&self, request: Request) -> Result<()> {
        let (sent, done) = oneshot::channel();
        let queue = if request.op == "reply" {
            &self.inner.priority
        } else {
            &self.inner.outbound
        };
        queue
            .send(Outbound { request, sent })
            .await
            .map_err(|_| anyhow!("Core transport closed"))?;
        done.await
            .context("Core writer closed; outcome unknown")?
            .map_err(anyhow::Error::msg)
    }
    pub async fn request(&self, op: &str, args: Value) -> Result<Value> {
        tokio::time::timeout(TIMEOUT, async {
            let slots = if op == "reply" {
                &self.inner.control_slots
            } else {
                &self.inner.slots
            };
            let _permit = slots.acquire().await?;
            let id = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
            let (sender, answer) = oneshot::channel();
            self.inner.pending.lock().unwrap().insert(id, sender);
            let _guard = PendingGuard {
                id,
                pending: self.inner.pending.clone(),
            };
            self.send(Request {
                id,
                op: op.into(),
                args,
            })
            .await?;
            answer
                .await
                .context("Core reply closed; outcome unknown")?
                .map_err(Into::into)
        })
        .await
        .map_err(|_| {
            anyhow!(Fault {
                code: "timeout_unknown".into(),
                message: "request timed out; outcome unknown, no automatic retry".into()
            })
        })?
    }
    pub async fn streams(&self) -> Result<Value> {
        self.request("streams", json!({})).await
    }
    pub async fn stream(&self, id: &str) -> Result<Value> {
        self.request("stream.get", json!({"stream":id})).await
    }
    pub async fn create_stream(&self, description: &str, parents: &[String]) -> Result<Value> {
        self.request(
            "stream.create",
            json!({"description":description,"parents":parents}),
        )
        .await
    }
    pub async fn resolve_stream(&self, stream: &str) -> Result<String> {
        if let Some(id) = self.inner.aliases.lock().await.get(stream).cloned() {
            return Ok(id);
        }
        let value = self.streams().await?;
        let list = value
            .as_array()
            .or_else(|| value["streams"].as_array())
            .context("Core omitted streams")?;
        let mut aliases = self.inner.aliases.lock().await;
        for entry in list {
            if let Some(id) = entry["id"].as_str() {
                aliases.insert(id.into(), id.into());
                if let Some(alias) = entry["alias"].as_str() {
                    aliases.insert(alias.into(), id.into());
                }
            }
        }
        aliases
            .get(stream)
            .cloned()
            .ok_or_else(|| anyhow!("unknown stream {stream}"))
    }
    pub async fn publish(
        &self,
        stream: &str,
        key: &str,
        payload: Vec<u8>,
        source_ts_ns: Option<u64>,
        upstream: BTreeMap<String, u64>,
    ) -> Result<PublishOutcome> {
        self.publish_tagged(stream, key, payload, source_ts_ns, upstream, None, None)
            .await
    }
    pub async fn publish_with_source_seq(
        &self,
        stream: &str,
        key: &str,
        payload: Vec<u8>,
        source_ts_ns: Option<u64>,
        upstream: BTreeMap<String, u64>,
        source_seq: Option<u64>,
    ) -> Result<PublishOutcome> {
        self.publish_tagged(
            stream,
            key,
            payload,
            source_ts_ns,
            upstream,
            None,
            source_seq,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn publish_tagged(
        &self,
        stream: &str,
        key: &str,
        payload: Vec<u8>,
        source_ts_ns: Option<u64>,
        upstream: BTreeMap<String, u64>,
        channel: Option<String>,
        source_seq: Option<u64>,
    ) -> Result<PublishOutcome> {
        if payload.len() > log_proto::MAX_PAYLOAD {
            bail!("payload exceeds protocol maximum");
        }
        let stream = self.resolve_stream(stream).await?;
        let args = json!({"stream":stream,"key":key,"payload":payload,"source_ts_ns":source_ts_ns,"upstream":upstream,"channel":channel,"source_seq":source_seq});
        if self.inner.kind == TransportKind::Udp {
            tokio::time::timeout(
                TIMEOUT,
                self.send(Request {
                    id: 0,
                    op: "publish".into(),
                    args,
                }),
            )
            .await
            .context("UDP local send timed out")??;
            Ok(PublishOutcome::LocalSent)
        } else {
            Ok(PublishOutcome::Accepted(serde_json::from_value(
                self.request("publish", args).await?,
            )?))
        }
    }
    /// Starts at Core's oldest retained record and waits indefinitely at the tail.
    pub async fn subscribe(&self, stream: &str) -> Result<Value> {
        let stream = self.resolve_stream(stream).await?;
        let mut subscriptions = self.inner.subscriptions.lock().await;
        if subscriptions.contains_key(&stream) {
            bail!("already subscribed to {stream}");
        }
        let (mut reader, mut writer, _) = socket(
            &self.inner.address,
            self.inner.kind,
            &self.inner.plugin,
            &self.inner.token,
            true,
        )
        .await?;
        writer
            .send(&Request {
                id: 1,
                op: "subscribe".into(),
                args: json!({"stream":stream}),
            })
            .await?;
        let result = match tokio::time::timeout(TIMEOUT, reader.receive()).await?? {
            Some(ServerMessage::Response {
                result,
                error: None,
                ..
            }) => result,
            Some(ServerMessage::Response { error: Some(e), .. }) => return Err(e.into()),
            _ => bail!("invalid subscription response"),
        };
        let events = self.inner.events.clone();
        let name = stream.clone();
        let task = tokio::spawn(async move {
            let _writer = writer;
            let reason = loop {
                let event = match reader.receive().await {
                    Ok(Some(ServerMessage::Record { record })) => Event::Record(record),
                    Ok(Some(ServerMessage::Gap {
                        stream,
                        epoch,
                        from,
                        to,
                        reason,
                    })) => Event::Gap {
                        stream,
                        epoch,
                        from,
                        to,
                        reason,
                    },
                    Ok(Some(ServerMessage::Response { error: Some(e), .. })) => {
                        break e.to_string()
                    }
                    Ok(Some(_)) => continue,
                    Ok(None) => break "Core subscription disconnected".into(),
                    Err(e) => break e.to_string(),
                };
                if events.send(event).await.is_err() {
                    return;
                }
            };
            let _ = events
                .send(Event::Disconnected {
                    stream: name,
                    reason,
                })
                .await;
        });
        subscriptions.insert(stream, task.abort_handle());
        Ok(result)
    }
    pub async fn unsubscribe(&self, stream: &str) -> Result<()> {
        let id = self.resolve_stream(stream).await?;
        if let Some(task) = self.inner.subscriptions.lock().await.remove(&id) {
            task.abort();
        }
        Ok(())
    }
    pub async fn reply_control(
        &self,
        call_id: u64,
        result: Value,
        error: Option<Fault>,
    ) -> Result<()> {
        self.request(
            "reply",
            json!({"call_id":call_id,"result":result,"error":error}),
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
