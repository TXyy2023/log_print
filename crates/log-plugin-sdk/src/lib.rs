//! Bounded plugin transport. An owned writer completes frames after caller cancellation.
use anyhow::{anyhow, bail, Context, Result};
use log_proto::{
    read_json, write_json, Fault, Hello, Record, Request, ServerMessage, EVENT_QUEUE, MAX_WIRE,
    PROTOCOL,
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
    io::{AsyncBufRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::{tcp::OwnedWriteHalf, TcpStream},
    sync::{mpsc, oneshot, watch, Mutex, Semaphore},
    task::AbortHandle,
};

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const OUTBOUND_CAPACITY: usize = 32;
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
struct Transport {
    pending: Pending,
    failure: std::sync::Mutex<Option<Fault>>,
    shutdown: watch::Sender<bool>,
    events: mpsc::Sender<Event>,
}
impl Transport {
    fn error(&self) -> Option<Fault> {
        self.failure.lock().unwrap().clone()
    }
    fn fail(&self, reason: impl ToString) {
        let fault = Fault {
            code: "connection_lost".into(),
            message: format!(
                "{}; unacknowledged operation outcomes may be unknown",
                reason.to_string()
            ),
        };
        {
            let mut failure = self.failure.lock().unwrap();
            if failure.is_some() {
                return;
            }
            *failure = Some(fault.clone());
        }
        self.shutdown.send_replace(true);
        for (_, reply) in std::mem::take(&mut *self.pending.lock().unwrap()) {
            let _ = reply.send(Err(fault.clone()));
        }
        // Never make failure notification block the RPC reader. Pending requests and
        // the closed control channel also expose failure when the event queue is full.
        let _ = self.events.try_send(Event::Disconnected {
            stream: "*".into(),
            reason: fault.message,
        });
    }
}
struct Inner {
    address: String,
    plugin: String,
    token: String,
    config: Value,
    outbound: mpsc::Sender<Vec<u8>>,
    priority: mpsc::Sender<Vec<u8>>,
    transport: Arc<Transport>,
    sequence: AtomicU64,
    slots: Semaphore,
    control_slots: Semaphore,
    events: mpsc::Sender<Event>,
    subscriptions: Mutex<BTreeMap<String, AbortHandle>>,
    rpc_reader: AbortHandle,
    rpc_writer: AbortHandle,
    timeout: Duration,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.transport.fail("SDK client closed");
        self.rpc_reader.abort();
        self.rpc_writer.abort();
        for handle in self.subscriptions.get_mut().values() {
            handle.abort();
        }
    }
}
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}
fn frame(request: &Request) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(request)?;
    if bytes.len() + 1 > MAX_WIRE {
        bail!("frame_too_large")
    }
    bytes.push(b'\n');
    Ok(bytes)
}
async fn socket(
    address: &str,
    plugin: &str,
    token: &str,
    events: bool,
) -> Result<(BufReader<tokio::net::tcp::OwnedReadHalf>, OwnedWriteHalf)> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let address: std::net::SocketAddr = address
            .parse()
            .context("Core address must be a loopback socket address")?;
        if !address.ip().is_loopback() {
            bail!("Core address must be loopback")
        }
        let stream = TcpStream::connect(address).await?;
        stream.set_nodelay(true)?;
        let (r, mut w) = stream.into_split();
        let mut r = BufReader::new(r);
        write_json(
            &mut w,
            &Hello {
                protocol: PROTOCOL.into(),
                plugin: plugin.into(),
                token: token.into(),
                events,
            },
        )
        .await?;
        match read_json::<_, ServerMessage>(&mut r).await? {
            Some(ServerMessage::Response {
                id: 0,
                error: None,
                result,
            }) if result["protocol"] == PROTOCOL => (),
            Some(ServerMessage::Response { error: Some(e), .. }) => return Err(e.into()),
            _ => bail!("invalid Core welcome"),
        }
        Ok((r, w))
    })
    .await
    .context("Core connection/handshake timed out")?
}
async fn writer_worker<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut ordinary: mpsc::Receiver<Vec<u8>>,
    mut priority: mpsc::Receiver<Vec<u8>>,
    transport: Arc<Transport>,
    timeout: Duration,
) {
    let mut stop = transport.shutdown.subscribe();
    loop {
        if *stop.borrow() {
            return;
        }
        let bytes = tokio::select! {
            biased;
            _=stop.changed()=>return,
            Some(bytes)=priority.recv()=>bytes,
            Some(bytes)=ordinary.recv()=>bytes,
            else=>return,
        };
        // This worker owns the full frame. Cancellation of the requesting future
        // cannot cancel write_all. A transport timeout closes BOTH socket halves.
        let result = tokio::select! {
            _=stop.changed()=>return,
            result=tokio::time::timeout(timeout,async {writer.write_all(&bytes).await?;writer.flush().await})=>result,
        };
        match result {
            Ok(Ok(())) => (),
            Ok(Err(e)) => {
                transport.fail(format!("RPC write failed: {e}"));
                return;
            }
            Err(_) => {
                transport.fail("RPC write deadline exceeded");
                return;
            }
        }
    }
}
async fn reader_worker<R: AsyncBufRead + Unpin>(
    mut reader: R,
    controls: mpsc::Sender<Control>,
    priority: mpsc::Sender<Vec<u8>>,
    transport: Arc<Transport>,
) {
    let mut stop = transport.shutdown.subscribe();
    loop {
        if *stop.borrow() {
            return;
        }
        let message = tokio::select! {_=stop.changed()=>return,message=read_json::<_,ServerMessage>(&mut reader)=>message};
        match message {
            Ok(Some(ServerMessage::Response { id, result, error })) => {
                if let Some(reply) = transport.pending.lock().unwrap().remove(&id) {
                    let _ = reply.send(match error {
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
                    let reply = Request {
                        id: 0,
                        op: "reply".into(),
                        args: json!({"call_id":call_id,"result":null,"error":{"code":"control_busy","message":"Plugin control queue is full or closed"}}),
                    };
                    // A full reserved queue means this connection cannot safely
                    // service control. Fail it rather than block reading replies.
                    let enqueued = frame(&reply)
                        .ok()
                        .is_some_and(|bytes| priority.try_send(bytes).is_ok());
                    if !enqueued {
                        transport.fail("control reply queue exhausted");
                        return;
                    }
                }
            }
            Ok(Some(_)) => {
                transport.fail("unexpected data on RPC connection");
                return;
            }
            Ok(None) => {
                transport.fail("Core RPC disconnected");
                return;
            }
            Err(e) => {
                transport.fail(format!("Core RPC framing error: {e}"));
                return;
            }
        }
    }
}
pub async fn connect_env() -> Result<(Client, mpsc::Receiver<Event>, mpsc::Receiver<Control>)> {
    let address = std::env::var("LOG_PRINT_CORE")
        .context("LOG_PRINT_CORE missing; launch plugin via log-print")?;
    let plugin = std::env::var("LOG_PRINT_PLUGIN")?;
    let token = std::env::var("LOG_PRINT_TOKEN")?;
    let config =
        serde_json::from_str(&std::env::var("LOG_PRINT_CONFIG").unwrap_or_else(|_| "{}".into()))?;
    Client::connect_with_config(&address, &plugin, &token, config).await
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
        let (reader, writer) = socket(address, plugin, token, false).await?;
        Ok(Self::from_parts(
            address,
            plugin,
            token,
            config,
            reader,
            writer,
            RPC_TIMEOUT,
        ))
    }
    fn from_parts<R, W>(
        address: &str,
        plugin: &str,
        token: &str,
        config: Value,
        reader: R,
        writer: W,
        timeout: Duration,
    ) -> (Self, mpsc::Receiver<Event>, mpsc::Receiver<Control>)
    where
        R: AsyncBufRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (outbound, ordinary_rx) = mpsc::channel(OUTBOUND_CAPACITY);
        let (priority, priority_rx) = mpsc::channel(OUTBOUND_CAPACITY);
        let (events, rx) = mpsc::channel(EVENT_QUEUE);
        let (controls, crx) = mpsc::channel(32);
        let (shutdown, _) = watch::channel(false);
        let transport = Arc::new(Transport {
            pending: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            failure: std::sync::Mutex::new(None),
            shutdown,
            events: events.clone(),
        });
        let rpc_writer = tokio::spawn(writer_worker(
            writer,
            ordinary_rx,
            priority_rx,
            transport.clone(),
            timeout,
        ));
        let rpc_reader = tokio::spawn(reader_worker(
            reader,
            controls,
            priority.clone(),
            transport.clone(),
        ));
        (
            Self {
                inner: Arc::new(Inner {
                    address: address.into(),
                    plugin: plugin.into(),
                    token: token.into(),
                    config,
                    outbound,
                    priority,
                    transport,
                    sequence: AtomicU64::new(1),
                    slots: Semaphore::new(32),
                    control_slots: Semaphore::new(32),
                    events,
                    subscriptions: Mutex::new(BTreeMap::new()),
                    rpc_reader: rpc_reader.abort_handle(),
                    rpc_writer: rpc_writer.abort_handle(),
                    timeout,
                }),
            },
            rx,
            crx,
        )
    }
    pub fn config(&self) -> &Value {
        &self.inner.config
    }
    pub async fn request(&self, op: &str, args: Value) -> Result<Value> {
        let mut stop = self.inner.transport.shutdown.subscribe();
        let operation = async {
            if let Some(error) = self.inner.transport.error() {
                return Err(error.into());
            }
            let priority = op == "reply";
            let slots = if priority {
                &self.inner.control_slots
            } else {
                &self.inner.slots
            };
            let _permit = slots.acquire().await?;
            if let Some(error) = self.inner.transport.error() {
                return Err(error.into());
            }
            let id = self.inner.sequence.fetch_add(1, Ordering::Relaxed);
            let bytes = frame(&Request {
                id,
                op: op.into(),
                args,
            })?;
            let (reply, answer) = oneshot::channel();
            self.inner
                .transport
                .pending
                .lock()
                .unwrap()
                .insert(id, reply);
            let _pending = PendingGuard {
                id,
                pending: self.inner.transport.pending.clone(),
            };
            let send = if priority {
                &self.inner.priority
            } else {
                &self.inner.outbound
            };
            send.send(bytes)
                .await
                .map_err(|_| anyhow!("RPC writer closed; operation outcome unknown"))?;
            answer
                .await
                .context("RPC reply channel closed; operation outcome unknown")?
                .map_err(Into::into)
        };
        tokio::select! {
            biased;
            result=tokio::time::timeout(self.inner.timeout,operation)=>match result {
                Ok(result)=>result,
                Err(_)=>Err(anyhow!(Fault{code:"timeout_unknown".into(),message:"RPC deadline includes queueing, transmission and response. An operation may still complete; do not automatically retry controls.".into()})),
            },
            _=stop.changed()=>Err(self.inner.transport.error().unwrap_or(Fault{code:"connection_lost".into(),message:"Core transport closed".into()}).into()),
        }
    }
    pub async fn publish(
        &self,
        stream: &str,
        key: &str,
        payload: Vec<u8>,
        source_ts_ns: Option<u64>,
        upstream: BTreeMap<String, u64>,
    ) -> Result<Record> {
        Ok(serde_json::from_value(self.request("publish",json!({"stream":stream,"key":key,"payload":payload,"source_ts_ns":source_ts_ns,"upstream":upstream})).await?)?)
    }
    /// Retains exactly one publication while its stream awaits explicit admin resume.
    pub async fn publish_retained(
        &self,
        stream: &str,
        key: &str,
        payload: Vec<u8>,
        source_ts_ns: Option<u64>,
        upstream: BTreeMap<String, u64>,
    ) -> Result<Record> {
        let mut reported = false;
        loop {
            match self
                .publish(stream, key, payload.clone(), source_ts_ns, upstream.clone())
                .await
            {
                Ok(record) => return Ok(record),
                Err(e)
                    if e.downcast_ref::<Fault>().is_some_and(|f| {
                        matches!(f.code.as_str(), "storage_blocked" | "commit_unknown")
                    }) =>
                {
                    if !reported {
                        eprintln!(
                            "{stream}: {e}; retaining current chunk, waiting for manual resume"
                        );
                        reported = true;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
    pub async fn subscribe(&self, stream: &str, from: u64) -> Result<()> {
        self.subscribe_epoch(stream, from, None).await
    }
    pub async fn subscribe_epoch(
        &self,
        stream: &str,
        from: u64,
        epoch: Option<&str>,
    ) -> Result<()> {
        tokio::time::timeout(
            self.inner.timeout,
            self.subscribe_epoch_inner(stream, from, epoch),
        )
        .await
        .context("subscription setup deadline exceeded")?
    }
    async fn subscribe_epoch_inner(
        &self,
        stream: &str,
        from: u64,
        epoch: Option<&str>,
    ) -> Result<()> {
        if let Some(error) = self.inner.transport.error() {
            return Err(error.into());
        }
        let mut subscriptions = self.inner.subscriptions.lock().await;
        if subscriptions.contains_key(stream) {
            bail!("already subscribed to {stream}")
        }
        let (mut reader, mut writer) = socket(
            &self.inner.address,
            &self.inner.plugin,
            &self.inner.token,
            true,
        )
        .await?;
        write_json(
            &mut writer,
            &Request {
                id: 1,
                op: "subscribe".into(),
                args: json!({"stream":stream,"from":from,"epoch":epoch}),
            },
        )
        .await?;
        match read_json::<_, ServerMessage>(&mut reader).await? {
            Some(ServerMessage::Response { error: None, .. }) => {}
            Some(ServerMessage::Response { error: Some(e), .. }) => return Err(e.into()),
            _ => bail!("invalid subscription response"),
        }
        let tx = self.inner.events.clone();
        let name = stream.to_owned();
        let transport = self.inner.transport.clone();
        let mut stop = transport.shutdown.subscribe();
        let task = tokio::spawn(async move {
            let _writer = writer;
            let reason = loop {
                if *stop.borrow() {
                    break transport
                        .error()
                        .map(|e| e.message)
                        .unwrap_or_else(|| "RPC transport closed".into());
                }
                let message = tokio::select! {
                    _=stop.changed()=>break transport.error().map(|e|e.message).unwrap_or_else(||"RPC transport closed".into()),
                    message=read_json::<_,ServerMessage>(&mut reader)=>message,
                };
                let event = match message {
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
                        break e.to_string();
                    }
                    Ok(None) => {
                        break "Core event connection closed; missing count is unknown".to_string();
                    }
                    Err(e) => {
                        break e.to_string();
                    }
                    _ => continue,
                };
                tokio::select! {
                    _=stop.changed()=>break transport.error().map(|e|e.message).unwrap_or_else(||"RPC transport closed".into()),
                    sent=tx.send(event)=>if sent.is_err(){return},
                }
            };
            drop(_writer);
            drop(reader);
            eprintln!("subscription {name}: {reason}");
            let _ = tx
                .send(Event::Disconnected {
                    stream: name,
                    reason,
                })
                .await;
        });
        subscriptions.insert(stream.into(), task.abort_handle());
        Ok(())
    }
    pub async fn unsubscribe(&self, stream: &str) -> Result<()> {
        if let Some(handle) = self.inner.subscriptions.lock().await.remove(stream) {
            handle.abort();
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
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, DuplexStream};

    fn bounded_client(
        timeout: Duration,
    ) -> (
        Client,
        DuplexStream,
        mpsc::Receiver<Event>,
        mpsc::Receiver<Control>,
    ) {
        let (client_socket, server) = tokio::io::duplex(64);
        let (reader, writer) = tokio::io::split(client_socket);
        let (client, events, controls) = Client::from_parts(
            "127.0.0.1:1",
            "test",
            "token",
            json!({}),
            BufReader::new(reader),
            writer,
            timeout,
        );
        (client, server, events, controls)
    }
    async fn answer<W: AsyncWrite + Unpin>(writer: &mut W, id: u64) -> Result<()> {
        write_json(
            writer,
            &ServerMessage::Response {
                id,
                result: json!({"ok":true}),
                error: None,
            },
        )
        .await
    }

    #[tokio::test]
    async fn cancelled_request_cannot_leave_a_partial_frame() -> Result<()> {
        let (client, server, _events, _controls) = bounded_client(Duration::from_secs(2));
        let (mut server_read, mut server_write) = tokio::io::split(server);
        let requester = client.clone();
        let cancelled = tokio::spawn(async move {
            requester
                .request("large", json!({"body":"x".repeat(8000)}))
                .await
        });
        // The 64-byte socket is now inside a frame, with most bytes still pending.
        let first = server_read.read_u8().await?;
        assert_eq!(first, b'{');
        cancelled.abort();
        let _ = cancelled.await;
        let server = tokio::spawn(async move {
            let reader = std::io::Cursor::new(vec![first]).chain(server_read);
            let mut reader = BufReader::new(reader);
            let complete: Request = read_json(&mut reader)
                .await?
                .context("cancelled frame vanished")?;
            assert_eq!(complete.op, "large");
            assert_eq!(complete.args["body"].as_str().unwrap().len(), 8000);
            let next: Request = read_json(&mut reader)
                .await?
                .context("next frame vanished")?;
            assert_eq!(next.op, "after");
            answer(&mut server_write, next.id).await?;
            Ok::<(), anyhow::Error>(())
        });
        assert_eq!(
            client.request("after", json!({})).await?,
            json!({"ok":true})
        );
        server.await??;
        assert!(client.inner.transport.pending.lock().unwrap().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn rpc_deadline_covers_slot_wait_and_blocked_write() -> Result<()> {
        let (client, _server, _events, _controls) = bounded_client(Duration::from_millis(80));
        let permits = client.inner.slots.acquire_many(32).await?;
        let before = tokio::time::Instant::now();
        let error = client.request("status", json!({})).await.unwrap_err();
        assert_eq!(
            error.downcast_ref::<Fault>().unwrap().code,
            "timeout_unknown"
        );
        assert!(before.elapsed() < Duration::from_millis(500));
        drop(permits);
        let before = tokio::time::Instant::now();
        let error = client
            .request("large", json!({"body":"x".repeat(8000)}))
            .await
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<Fault>().map(|e| e.code.as_str()),
            Some("timeout_unknown" | "connection_lost")
        ));
        assert!(before.elapsed() < Duration::from_millis(500));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(client.inner.transport.pending.lock().unwrap().is_empty());
        let before = tokio::time::Instant::now();
        assert!(client.request("status", json!({})).await.is_err());
        assert!(before.elapsed() < Duration::from_millis(40));
        Ok(())
    }

    #[tokio::test]
    async fn full_control_queue_does_not_block_rpc_response_reader() -> Result<()> {
        let (client, server, _events, _controls) = bounded_client(Duration::from_secs(2));
        let (_server_read, mut server_write) = tokio::io::split(server);
        let requester = client.clone();
        let request = tokio::spawn(async move { requester.request("probe", json!({})).await });
        while client.inner.transport.pending.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
        // Never consume client->server bytes. Busy replies therefore block the
        // writer, while the independent reader must continue receiving responses.
        for call_id in 0..40 {
            write_json(
                &mut server_write,
                &ServerMessage::Control {
                    call_id,
                    method: "ignored".into(),
                    args: json!({}),
                },
            )
            .await?;
        }
        answer(&mut server_write, 1).await?;
        let result = tokio::time::timeout(Duration::from_millis(500), request).await???;
        assert_eq!(result, json!({"ok":true}));
        Ok(())
    }

    #[tokio::test]
    async fn exhausted_control_reply_reserve_closes_transport_and_pending() -> Result<()> {
        let (client, server, mut events, mut controls) = bounded_client(Duration::from_secs(2));
        let (_server_read, mut server_write) = tokio::io::split(server);
        let requester = client.clone();
        let request = tokio::spawn(async move {
            requester
                .request("large", json!({"body":"x".repeat(8000)}))
                .await
        });
        while client.inner.transport.pending.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
        for call_id in 0..100 {
            if write_json(
                &mut server_write,
                &ServerMessage::Control {
                    call_id,
                    method: "ignored".into(),
                    args: json!({}),
                },
            )
            .await
            .is_err()
            {
                break;
            }
        }
        let error = tokio::time::timeout(Duration::from_millis(500), request)
            .await??
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<Fault>().unwrap().code,
            "connection_lost"
        );
        assert!(client.inner.transport.pending.lock().unwrap().is_empty());
        let event = tokio::time::timeout(Duration::from_millis(500), events.recv())
            .await?
            .unwrap();
        assert!(matches!(event, Event::Disconnected { .. }));
        while controls.recv().await.is_some() {}
        assert!(client.request("after", json!({})).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn client_drop_closes_both_halves_without_a_task_cycle() -> Result<()> {
        let (client, mut server, _events, _controls) = bounded_client(Duration::from_secs(2));
        drop(client);
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_millis(500), server.read(&mut byte)).await??,
            0
        );
        Ok(())
    }
}
