mod storage;
use anyhow::{anyhow, bail, Context, Result};
use log_proto::{
    now_ns, read_json, write_json, Config, CoreOptions, Fault, Hello, Record, Request,
    RuntimeConfig, SaveOptions, ServerMessage, MAX_PAYLOAD, PROTOCOL,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, RwLock,
    },
};
use storage::Store;
use tokio::{
    io::BufReader,
    net::TcpStream,
    sync::{mpsc, oneshot, Notify},
};

fn fault(code: &str, message: impl ToString) -> anyhow::Error {
    Fault {
        code: code.into(),
        message: message.to_string(),
    }
    .into()
}
fn wire_error(e: anyhow::Error) -> Fault {
    e.downcast_ref::<Fault>().cloned().unwrap_or(Fault {
        code: "invalid_request".into(),
        message: format!("{e:#}"),
    })
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 100
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_- .".contains(&b))
        && !id.contains(' ')
}
fn check_options(c: &CoreOptions) -> Result<()> {
    if c.buffer_bytes < MAX_PAYLOAD
        || c.buffer_bytes > 1024 * 1024 * 1024
        || c.buffer_records == 0
        || c.buffer_records > 1_000_000
        || c.max_payload_bytes == 0
        || c.max_payload_bytes > MAX_PAYLOAD
        || c.read_batch_records == 0
        || c.read_batch_records > 64
        || c.queue_records == 0
        || c.queue_records > 1024
    {
        bail!("Core limits out of range")
    };
    Ok(())
}
pub fn validate(config: &Config) -> Result<()> {
    check_options(&config.core)?;
    if config.plugins.len() > 128 {
        bail!("at most 128 plugins")
    }
    let mut plugins = BTreeSet::new();
    let mut streams = BTreeMap::new();
    for p in &config.plugins {
        if !valid_id(&p.id) || !plugins.insert(p.id.clone()) || p.id == "__admin__" {
            bail!("invalid or duplicate plugin id: {}", p.id)
        }
        if p.reads.len() > 128 || p.streams.len() > 128 {
            bail!("too many streams or subscriptions")
        }
        for s in &p.streams {
            if !valid_id(&s.id)
                || streams
                    .insert(s.id.clone(), (p.id.clone(), s.parents.clone()))
                    .is_some()
            {
                bail!("invalid or duplicate stream id: {}", s.id)
            }
            let save = CoreOptions::default()
                .save
                .overlay(&config.core.save)
                .overlay(&p.save)
                .overlay(&s.save);
            if save.enabled.unwrap_or(false)
                && (save.directory.as_ref().is_none_or(|d| d.is_empty())
                    || save.file_bytes.unwrap_or(0) < 262144
                    || save.total_bytes.unwrap_or(0)
                        < save.file_bytes.unwrap_or(0).saturating_mul(2))
            {
                bail!("invalid save options for {}", s.id)
            }
        }
    }
    if streams.len() > 256 {
        bail!("at most 256 streams")
    }
    for p in &config.plugins {
        for r in &p.reads {
            if !streams.contains_key(r) {
                bail!("{} reads unknown stream {r}", p.id)
            }
        }
        for s in &p.streams {
            let parents: BTreeSet<_> = s.parents.iter().collect();
            if parents.len() != s.parents.len() || parents.len() > 32 {
                bail!("duplicate or too many parents")
            }
            for parent in &s.parents {
                if !streams.contains_key(parent) || !p.reads.contains(parent) {
                    bail!(
                        "{} parent {parent} must exist and appear in owner reads",
                        s.id
                    )
                }
            }
        }
    }
    // A plugin may publish every stream it owns after reading any declared input.
    // Include that process-level dependency even if parents were omitted/misdeclared.
    for (owner, parents) in streams.values_mut() {
        for read in &config
            .plugins
            .iter()
            .find(|p| p.id == *owner)
            .unwrap()
            .reads
        {
            if !parents.contains(read) {
                parents.push(read.clone());
            }
        }
    }
    fn visit(
        id: &str,
        map: &BTreeMap<String, (String, Vec<String>)>,
        path: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        }
        if !path.insert(id.into()) {
            bail!("stream cycle at {id}")
        }
        for parent in &map[id].1 {
            visit(parent, map, path, done)?
        }
        path.remove(id);
        done.insert(id.into());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for id in streams.keys() {
        visit(id, &streams, &mut BTreeSet::new(), &mut done)?
    }
    // Self-reading would create a process feedback path even when its declared stream DAG does not.
    for p in &config.plugins {
        for r in &p.reads {
            if streams[r].0 == p.id {
                bail!("plugin {} cannot subscribe to its own output {r}", p.id)
            }
        }
    }
    Ok(())
}
struct Stream {
    id: String,
    owner: String,
    parents: Vec<String>,
    identity: String,
    epoch: String,
    head: u64,
    buffer: VecDeque<Record>,
    bytes: usize,
    save: SaveOptions,
    store: Option<Store>,
    blocked: Option<String>,
    received: u64,
    uncommitted_key: Option<String>,
    evicted: u64,
}
impl Stream {
    fn trim(&mut self, options: &CoreOptions) {
        while self.buffer.len() > options.buffer_records || self.bytes > options.buffer_bytes {
            if let Some(record) = self.buffer.pop_front() {
                self.bytes -= record.payload.len();
                self.evicted = record.seq;
            }
        }
    }
    fn push(&mut self, record: Record, options: &CoreOptions) {
        self.head = record.seq;
        self.bytes += record.payload.len();
        self.buffer.push_back(record);
        self.trim(options);
    }
    fn oldest(&self) -> u64 {
        if self.store.is_some() {
            1
        } else {
            self.buffer.front().map(|r| r.seq).unwrap_or(self.head + 1)
        }
    }
    fn status(&self) -> Value {
        json!({"id":self.id,"owner":self.owner,"parents":self.parents,"epoch":self.epoch,"head":self.head,"oldest":self.oldest(),"buffer_oldest":self.buffer.front().map(|r|r.seq),"buffer_records":self.buffer.len(),"buffer_bytes":self.bytes,"save":self.save,"blocked":self.blocked,"received":self.received,"uncommitted_key":self.uncommitted_key,"uncommitted_outcome":if self.uncommitted_key.is_some(){Some("unknown_or_not_committed")}else{None},"evicted_through":self.evicted,"storage_bytes":self.store.as_ref().and_then(|s|s.disk_bytes().ok())})
    }
    fn publish(
        &mut self,
        args: Publish,
        options: &CoreOptions,
        upstream_epochs: BTreeMap<String, String>,
    ) -> Result<Record> {
        if args.payload.len() > options.max_payload_bytes
            || args.key.is_empty()
            || args.key.len() > 256
        {
            return Err(fault("limit", "payload or idempotency key exceeds limit"));
        }
        let known = if let Some(s) = &self.store {
            s.by_key(&args.key)
                .map_err(|e| fault("history_unavailable", e))?
        } else {
            self.buffer.iter().find(|r| r.key == args.key).cloned()
        };
        if let Some(record) = known {
            if record.payload != args.payload
                || record.source_ts_ns != args.source_ts_ns
                || record.upstream != args.upstream
                || record.upstream_epochs != upstream_epochs
            {
                return Err(fault("key_conflict", "same key has different content"));
            }
            return Ok(record);
        }
        if let Some(reason) = &self.blocked {
            return Err(fault("storage_blocked", reason));
        }
        self.received += 1;
        let record = Record {
            stream: self.id.clone(),
            epoch: self.epoch.clone(),
            seq: self.head + 1,
            key: args.key,
            payload: args.payload,
            source_ts_ns: args.source_ts_ns,
            observed_ts_ns: now_ns(),
            upstream: args.upstream,
            upstream_epochs,
            durability: if self.save.enabled.unwrap_or(false) {
                "saved"
            } else {
                "buffered"
            }
            .into(),
        };
        if let Some(s) = &mut self.store {
            if let Err(e) = s.append(&record) {
                let msg = format!("{e:#}");
                self.blocked = Some(msg.clone());
                self.uncommitted_key = Some(record.key.clone());
                return Err(fault(
                    if msg.contains("commit_unknown") {
                        "commit_unknown"
                    } else {
                        "storage_blocked"
                    },
                    msg,
                ));
            }
        } else if self.save.enabled.unwrap_or(false) {
            return Err(fault(
                "storage_blocked",
                "saved stream storage is unavailable",
            ));
        }
        self.push(record.clone(), options);
        Ok(record)
    }
    fn read(&self, from: u64, limit: usize, epoch: Option<&str>) -> Result<Value> {
        if self.save.enabled.unwrap_or(false) && self.store.is_none() {
            return Err(fault(
                "history_unavailable",
                self.blocked
                    .as_deref()
                    .unwrap_or("saved history is unavailable"),
            ));
        }
        if epoch.is_some_and(|e| e != self.epoch) {
            return Err(fault(
                "epoch_mismatch",
                "cursor belongs to a different stream incarnation",
            ));
        }
        let from = if from == 0 { self.head + 1 } else { from };
        if from > self.head + 1 {
            return Err(fault("cursor_ahead", format!("head is {}", self.head)));
        }
        let oldest = self.oldest();
        let start = from.max(oldest);
        let records = if self.buffer.front().is_some_and(|r| start >= r.seq) {
            self.buffer
                .iter()
                .filter(|r| r.seq >= start)
                .take(limit)
                .cloned()
                .collect::<Vec<_>>()
        } else if let Some(s) = &self.store {
            s.read(start, limit)
                .map_err(|e| fault("history_unavailable", e))?
        } else if self.save.enabled.unwrap_or(false) && from <= self.head {
            return Err(fault(
                "history_unavailable",
                self.blocked.as_deref().unwrap_or("storage unavailable"),
            ));
        } else {
            Vec::new()
        };
        let mut bounded = Vec::new();
        let mut bytes = 0;
        for r in records {
            let n = serde_json::to_vec(&r)?.len();
            if bytes + n > 512 * 1024 && !bounded.is_empty() {
                break;
            }
            bytes += n;
            bounded.push(r);
        }
        let mut expected = start;
        for r in &bounded {
            if r.seq != expected {
                return Err(fault(
                    "history_gap",
                    format!("expected {expected}, found {}", r.seq),
                ));
            }
            expected += 1;
        }
        if start <= self.head && bounded.is_empty() {
            return Err(fault(
                "history_unavailable",
                "record within known head is missing",
            ));
        }
        let next = bounded.last().map(|r| r.seq + 1).unwrap_or(start);
        Ok(
            json!({"stream":self.id,"epoch":self.epoch,"from":from,"next":next,"head":self.head,"oldest":oldest,"records":bounded,"gap":if from<oldest {json!({"from":from,"to":oldest-1,"reason":"buffer_overwritten"})}else{Value::Null}}),
        )
    }
    fn resume(&mut self) -> Result<Value> {
        if !self.save.enabled.unwrap_or(false) {
            return Err(fault("not_saved", "stream does not use persistence"));
        }
        self.store.take();
        match Store::open(&self.id, &self.identity, &self.save) {
            Ok(store) => {
                self.epoch = store.epoch.clone();
                self.head = store.head;
                self.store = Some(store);
                self.buffer.clear();
                self.bytes = 0;
                self.blocked = None;
                self.uncommitted_key = None;
                Ok(self.status())
            }
            Err(e) => {
                self.blocked = Some(format!("{e:#}"));
                Err(fault("storage_blocked", e))
            }
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    stream: String,
    key: String,
    payload: Vec<u8>,
    #[serde(default)]
    source_ts_ns: Option<u64>,
    #[serde(default)]
    upstream: BTreeMap<String, u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    stream: String,
    #[serde(default = "one")]
    from: u64,
    limit: Option<usize>,
    epoch: Option<String>,
}
fn one() -> u64 {
    1
}
struct ConnectionInfo {
    generation: u64,
    sender: mpsc::Sender<ServerMessage>,
}
struct ConnectionLease {
    core: Arc<Core>,
    plugin: String,
    generation: u64,
}
impl Drop for ConnectionLease {
    fn drop(&mut self) {
        let core = self.core.clone();
        let plugin = self.plugin.clone();
        let generation = self.generation;
        tokio::spawn(async move {
            let mut connections = core.connections.lock().await;
            if connections
                .get(&plugin)
                .is_some_and(|c| c.generation == generation)
            {
                connections.remove(&plugin);
            }
        });
    }
}
type CallResult = Result<Value, Fault>;
struct PendingCall {
    target: String,
    sender: oneshot::Sender<CallResult>,
}
pub struct Core {
    runtime: RuntimeConfig,
    options: RwLock<CoreOptions>,
    streams: BTreeMap<String, Arc<Mutex<Stream>>>,
    notifies: BTreeMap<String, Arc<Notify>>,
    connections: tokio::sync::Mutex<BTreeMap<String, ConnectionInfo>>,
    reports: tokio::sync::Mutex<BTreeMap<String, Value>>,
    calls: tokio::sync::Mutex<BTreeMap<u64, PendingCall>>,
    sequence: AtomicU64,
    event_connections: AtomicUsize,
}
impl Core {
    pub fn new(mut runtime: RuntimeConfig) -> Result<Arc<Self>> {
        runtime.config.core.save = CoreOptions::default()
            .save
            .overlay(&runtime.config.core.save);
        validate(&runtime.config)?;
        let mut streams = BTreeMap::new();
        let mut notifies = BTreeMap::new();
        for plugin in &runtime.config.plugins {
            for spec in &plugin.streams {
                let save = runtime
                    .config
                    .core
                    .save
                    .overlay(&plugin.save)
                    .overlay(&spec.save);
                let identity = serde_json::to_string(
                    &json!({"stream":spec.id,"owner":plugin.id,"parents":spec.parents,"protocol":PROTOCOL}),
                )?;
                let mut s = Stream {
                    id: spec.id.clone(),
                    owner: plugin.id.clone(),
                    parents: spec.parents.clone(),
                    identity,
                    epoch: uuid::Uuid::new_v4().to_string(),
                    head: 0,
                    buffer: VecDeque::new(),
                    bytes: 0,
                    save,
                    store: None,
                    blocked: None,
                    received: 0,
                    uncommitted_key: None,
                    evicted: 0,
                };
                if s.save.enabled.unwrap_or(false) {
                    if let Err(e) = s.resume() {
                        eprintln!("{}: {e}", s.id);
                    }
                }
                streams.insert(spec.id.clone(), Arc::new(Mutex::new(s)));
                notifies.insert(spec.id.clone(), Arc::new(Notify::new()));
            }
        }
        Ok(Arc::new(Self {
            options: RwLock::new(runtime.config.core.clone()),
            runtime,
            streams,
            notifies,
            connections: Default::default(),
            reports: Default::default(),
            calls: Default::default(),
            sequence: AtomicU64::new(1),
            event_connections: AtomicUsize::new(0),
        }))
    }
    fn stream(&self, id: &str) -> Result<Arc<Mutex<Stream>>> {
        self.streams
            .get(id)
            .cloned()
            .ok_or_else(|| fault("unknown_stream", id))
    }
    fn can_read(&self, plugin: &str, stream: &str) -> Result<()> {
        if plugin == "__admin__"
            || self
                .runtime
                .config
                .plugins
                .iter()
                .any(|p| p.id == plugin && p.reads.iter().any(|s| s == stream))
        {
            Ok(())
        } else {
            Err(fault("permission_denied", "stream is not in plugin reads"))
        }
    }
    async fn read(&self, plugin: &str, args: Value) -> Result<Value> {
        let r: Read = serde_json::from_value(args)?;
        self.can_read(plugin, &r.stream)?;
        let limit = r
            .limit
            .unwrap_or(self.options.read().unwrap().read_batch_records);
        if limit == 0 || limit > 64 {
            return Err(fault("limit", "read limit must be 1..64"));
        }
        let stream = self.stream(&r.stream)?;
        tokio::task::spawn_blocking(move || {
            stream
                .lock()
                .unwrap()
                .read(r.from, limit, r.epoch.as_deref())
        })
        .await?
    }
    async fn command(self: &Arc<Self>, plugin: &str, op: &str, args: Value) -> Result<Value> {
        match op {
            "status" => {
                let streams = self.streams.values().cloned().collect::<Vec<_>>();
                let states = tokio::task::spawn_blocking(move || {
                    streams
                        .iter()
                        .map(|s| s.lock().unwrap().status())
                        .collect::<Vec<_>>()
                })
                .await?;
                let connections = self.connections.lock().await;
                let reports = self.reports.lock().await;
                let plugins=self.runtime.config.plugins.iter().map(|p|json!({"id":p.id,"connected":connections.contains_key(&p.id),"report":reports.get(&p.id)})).collect::<Vec<_>>();
                Ok(
                    json!({"pid":std::process::id(),"protocol":PROTOCOL,"streams":states,"plugins":plugins,"config":self.runtime.config,"effective_core":*self.options.read().unwrap()}),
                )
            }
            "read" => self.read(plugin, args).await,
            "publish" => {
                let p: Publish = serde_json::from_value(args)?;
                let stream = self.stream(&p.stream)?;
                let (owner, parents) = self
                    .runtime
                    .config
                    .plugins
                    .iter()
                    .find_map(|owner| {
                        owner
                            .streams
                            .iter()
                            .find(|s| s.id == p.stream)
                            .map(|s| (owner.id.clone(), s.parents.clone()))
                    })
                    .context("stream metadata missing")?;
                if plugin != owner {
                    return Err(fault("permission_denied", "only stream owner may publish"));
                }
                if p.upstream.len() != parents.len()
                    || parents.iter().any(|s| !p.upstream.contains_key(s))
                {
                    return Err(fault(
                        "invalid_parents",
                        "upstream must include exactly declared parents",
                    ));
                }
                let upstream = p
                    .upstream
                    .iter()
                    .map(|(id, seq)| Ok((id.clone(), *seq, self.stream(id)?)))
                    .collect::<Result<Vec<_>>>()?;
                let upstream_epochs = tokio::task::spawn_blocking(move || {
                    let mut epochs = BTreeMap::new();
                    for (id, seq, parent) in upstream {
                        let parent = parent.lock().unwrap();
                        if seq == 0 || seq > parent.head {
                            return Err(fault("invalid_parent_cursor", id));
                        };
                        epochs.insert(id, parent.epoch.clone());
                    }
                    Ok(epochs)
                })
                .await??;
                let notify = self.notifies[&p.stream].clone();
                let options = self.options.read().unwrap().clone();
                let record = tokio::task::spawn_blocking(move || {
                    stream.lock().unwrap().publish(p, &options, upstream_epochs)
                })
                .await??;
                notify.notify_waiters();
                Ok(serde_json::to_value(record)?)
            }
            "resume" => {
                if plugin != "__admin__" {
                    return Err(fault("permission_denied", "admin required"));
                }
                let id = args["stream"].as_str().context("stream required")?;
                let stream = self.stream(id)?;
                let result =
                    tokio::task::spawn_blocking(move || stream.lock().unwrap().resume()).await??;
                self.notifies[id].notify_waiters();
                Ok(result)
            }
            "config.patch" => {
                if plugin != "__admin__" {
                    return Err(fault("permission_denied", "admin required"));
                }
                let mut options = self.options.read().unwrap().clone();
                for (key, value) in args.as_object().context("object required")? {
                    let n = value.as_u64().context("positive integer required")? as usize;
                    match key.as_str() {
                        "buffer_bytes" => options.buffer_bytes = n,
                        "buffer_records" => options.buffer_records = n,
                        "read_batch_records" => options.read_batch_records = n,
                        _ => {
                            return Err(fault(
                                "restart_required",
                                format!("{key} requires configuration file and restart"),
                            ))
                        }
                    }
                }
                check_options(&options)?;
                *self.options.write().unwrap() = options.clone();
                for stream in self.streams.values() {
                    stream.lock().unwrap().trim(&options)
                }
                Ok(json!({"effective":options,"source":"runtime override","persisted":false}))
            }
            "report" => {
                if serde_json::to_vec(&args)?.len() > 16384 {
                    return Err(fault("limit", "report too large"));
                }
                self.reports.lock().await.insert(plugin.into(), args);
                Ok(json!({"accepted":true}))
            }
            "control" => {
                if plugin != "__admin__" {
                    return Err(fault("permission_denied", "admin required"));
                }
                let target = args["target"]
                    .as_str()
                    .context("target required")?
                    .to_string();
                let method = args["method"]
                    .as_str()
                    .context("method required")?
                    .to_string();
                let call_id = self.sequence.fetch_add(1, Ordering::Relaxed);
                let (sender, receiver) = oneshot::channel();
                let connections = self.connections.lock().await;
                let connection = connections
                    .get(&target)
                    .ok_or_else(|| fault("not_connected", &target))?;
                if self.calls.lock().await.len() >= 128 {
                    return Err(fault("busy", "too many control calls"));
                }
                self.calls.lock().await.insert(
                    call_id,
                    PendingCall {
                        target: target.clone(),
                        sender,
                    },
                );
                if connection
                    .sender
                    .try_send(ServerMessage::Control {
                        call_id,
                        method,
                        args: args.get("args").cloned().unwrap_or(json!({})),
                    })
                    .is_err()
                {
                    self.calls.lock().await.remove(&call_id);
                    return Err(fault("control_busy", target));
                }
                drop(connections);
                let result =
                    tokio::time::timeout(std::time::Duration::from_secs(10), receiver).await;
                self.calls.lock().await.remove(&call_id);
                match result {
                    Ok(Ok(value)) => value.map_err(Into::into),
                    _ => Err(fault(
                        "control_timeout",
                        "plugin did not confirm within 10s; outcome unknown",
                    )),
                }
            }
            "reply" => {
                let id = args["call_id"].as_u64().context("call_id required")?;
                let mut calls = self.calls.lock().await;
                if let Some(call) = calls.get(&id) {
                    if call.target != plugin {
                        return Err(fault("permission_denied", "control reply owner mismatch"));
                    }
                }
                if let Some(call) = calls.remove(&id) {
                    let error: Option<Fault> =
                        serde_json::from_value(args.get("error").cloned().unwrap_or(Value::Null))?;
                    let _ = call.sender.send(match error {
                        Some(e) => Err(e),
                        None => Ok(args.get("result").cloned().unwrap_or(Value::Null)),
                    });
                }
                Ok(json!({"accepted":true}))
            }
            _ => Err(fault("unsupported_operation", op)),
        }
    }
    async fn subscription(
        self: &Arc<Self>,
        plugin: &str,
        args: Value,
        reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
    ) -> Result<()> {
        let r: Read = serde_json::from_value(args)?;
        let notify = self
            .notifies
            .get(&r.stream)
            .context("unknown stream")?
            .clone();
        let mut next = r.from;
        let mut epoch = r.epoch;
        loop {
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let batch = self
                .read(
                    plugin,
                    json!({"stream":r.stream,"from":next,"epoch":epoch,"limit":r.limit}),
                )
                .await?;
            next = batch["next"].as_u64().unwrap();
            epoch = Some(batch["epoch"].as_str().unwrap().into());
            if let Some(gap) = batch["gap"].as_object() {
                write_json(
                    writer,
                    &ServerMessage::Gap {
                        stream: r.stream.clone(),
                        epoch: epoch.clone().unwrap(),
                        from: gap["from"].as_u64().unwrap(),
                        to: gap["to"].as_u64().unwrap(),
                        reason: gap["reason"].as_str().unwrap().into(),
                    },
                )
                .await?;
            }
            let records: Vec<Record> = serde_json::from_value(batch["records"].clone())?;
            if records.is_empty() {
                tokio::select! { _=&mut notified=>{}, message=read_json::<_,Request>(reader)=>{if message?.is_some(){bail!("event connection accepts a single subscription")};return Ok(())} }
            } else {
                for record in records {
                    write_json(writer, &ServerMessage::Record { record }).await?;
                }
            }
        }
    }
    pub async fn connection(self: Arc<Self>, stream: TcpStream) -> Result<()> {
        stream.set_nodelay(true)?;
        let (r, mut w) = stream.into_split();
        let mut r = BufReader::new(r);
        let hello: Hello =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_json(&mut r))
                .await??
                .context("missing hello")?;
        let auth = if hello.protocol != PROTOCOL {
            Err(fault("version_mismatch", format!("requires {PROTOCOL}")))
        } else if (hello.plugin == "__admin__" && hello.token == self.runtime.admin_token)
            || self
                .runtime
                .plugin_tokens
                .get(&hello.plugin)
                .is_some_and(|t| *t == hello.token)
        {
            Ok(())
        } else {
            Err(fault(
                "authentication_failed",
                "invalid plugin identity/token",
            ))
        };
        if let Err(e) = auth {
            write_json(
                &mut w,
                &ServerMessage::Response {
                    id: 0,
                    result: Value::Null,
                    error: Some(wire_error(e)),
                },
            )
            .await?;
            return Ok(());
        }
        let generation = self.sequence.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel(self.options.read().unwrap().queue_records);
        let mut lease = None;
        if !hello.events && hello.plugin != "__admin__" {
            let mut connections = self.connections.lock().await;
            if connections.contains_key(&hello.plugin) {
                write_json(
                    &mut w,
                    &ServerMessage::Response {
                        id: 0,
                        result: Value::Null,
                        error: Some(Fault {
                            code: "already_connected".into(),
                            message: "plugin already has an active RPC connection".into(),
                        }),
                    },
                )
                .await?;
                return Ok(());
            }
            connections.insert(
                hello.plugin.clone(),
                ConnectionInfo {
                    generation,
                    sender: tx.clone(),
                },
            );
            lease = Some(ConnectionLease {
                core: self.clone(),
                plugin: hello.plugin.clone(),
                generation,
            });
        }
        write_json(
            &mut w,
            &ServerMessage::Response {
                id: 0,
                result: json!({"protocol":PROTOCOL,"plugin":hello.plugin}),
                error: None,
            },
        )
        .await?;
        if hello.events {
            let active = self.event_connections.fetch_add(1, Ordering::SeqCst);
            let result = async {
                if active >= 1024 {
                    bail!("too many event connections")
                }
                let request: Request = read_json(&mut r)
                    .await?
                    .context("subscription request missing")?;
                let check = if request.op != "subscribe" {
                    Err(fault(
                        "invalid_request",
                        "events connection requires subscribe",
                    ))
                } else {
                    self.read(&hello.plugin, request.args.clone()).await
                };
                let response = match check {
                    Ok(ref v) => ServerMessage::Response {
                        id: request.id,
                        result: json!({"epoch":v["epoch"],"head":v["head"],"subscribed":true}),
                        error: None,
                    },
                    Err(e) => {
                        write_json(
                            &mut w,
                            &ServerMessage::Response {
                                id: request.id,
                                result: Value::Null,
                                error: Some(wire_error(e)),
                            },
                        )
                        .await?;
                        return Ok(());
                    }
                };
                write_json(&mut w, &response).await?;
                if let Err(e) = self
                    .subscription(&hello.plugin, request.args, &mut r, &mut w)
                    .await
                {
                    let _ = write_json(
                        &mut w,
                        &ServerMessage::Response {
                            id: 0,
                            result: Value::Null,
                            error: Some(wire_error(e)),
                        },
                    )
                    .await;
                }
                Ok(())
            }
            .await;
            self.event_connections.fetch_sub(1, Ordering::SeqCst);
            return result;
        }
        let writer = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                write_json(&mut w, &message).await?;
            }
            Ok::<(), anyhow::Error>(())
        });
        let result = async {
            while let Some(request) = read_json::<_, Request>(&mut r).await? {
                let response = match self.command(&hello.plugin, &request.op, request.args).await {
                    Ok(result) => ServerMessage::Response {
                        id: request.id,
                        result,
                        error: None,
                    },
                    Err(e) => ServerMessage::Response {
                        id: request.id,
                        result: Value::Null,
                        error: Some(wire_error(e)),
                    },
                };
                tx.send(response)
                    .await
                    .map_err(|_| anyhow!("RPC writer closed"))?;
            }
            Ok(())
        }
        .await;
        drop(lease);
        drop(tx);
        writer.abort();
        result
    }
}
