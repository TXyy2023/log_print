//! Memory-only stream registry. Transport and framing live in log-proto.
use anyhow::{bail, Context, Result};
use log_proto::{
    now_ns, Config, Fault, Record, Role, RuntimeConfig, ServerConnection, ServerMessage,
    TransportKind, MAX_DATAGRAM, MAX_PAYLOAD, PROTOCOL,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, Notify};
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
pub fn validate(config: &Config) -> Result<()> {
    let c = &config.core;
    if c.buffer_bytes == 0
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
    if config.plugins.len() > 128 {
        bail!("at most 128 configured plugins")
    };
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for p in &config.plugins {
        if !valid_id(&p.id) || p.id == "__admin__" || !ids.insert(&p.id) {
            bail!("invalid or duplicate plugin id: {}", p.id)
        };
        if p.streams.len() > 1 {
            bail!("each plugin may own at most one stream")
        };
        if p.role == Role::Input && !p.reads.is_empty() {
            bail!("Input cannot subscribe; use an Output plugin")
        };
        if p.reads.len() > 128 {
            bail!("too many configured subscriptions")
        };
        for s in &p.streams {
            if !valid_id(&s.id) || aliases.insert(s.id.clone(), p.id.clone()).is_some() {
                bail!("invalid or duplicate stream alias: {}", s.id)
            };
            if s.description.len() > 4096 {
                bail!("stream description exceeds 4096 bytes")
            };
            if s.parents.len() > 32
                || s.parents.iter().collect::<BTreeSet<_>>().len() != s.parents.len()
            {
                bail!("duplicate or too many parents")
            };
            if p.role == Role::Input && !s.parents.is_empty() {
                bail!("Input stream cannot have parents")
            }
        }
    }
    for p in &config.plugins {
        for r in &p.reads {
            if !aliases.contains_key(r) && uuid::Uuid::parse_str(r).is_err() {
                bail!("{} reads unknown stream alias {r}", p.id)
            };
            if aliases.get(r) == Some(&p.id) {
                bail!("plugin cannot subscribe to its own stream")
            }
        }
        for s in &p.streams {
            for parent in &s.parents {
                if !p.reads.contains(parent) {
                    bail!("parent must appear in owner reads")
                }
            }
        }
    }
    // Reject configured process feedback cycles, including dependencies omitted from parents.
    fn visit(
        id: &str,
        config: &Config,
        aliases: &BTreeMap<String, String>,
        path: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(id) {
            return Ok(());
        };
        if !path.insert(id.into()) {
            bail!("stream cycle at plugin {id}")
        };
        if let Some(p) = config.plugins.iter().find(|p| p.id == id) {
            for r in &p.reads {
                if let Some(owner) = aliases.get(r) {
                    visit(owner, config, aliases, path, done)?
                }
            }
        };
        path.remove(id);
        done.insert(id.into());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for p in &config.plugins {
        visit(&p.id, config, &aliases, &mut BTreeSet::new(), &mut done)?
    }
    Ok(())
}
struct Stream {
    id: String,
    alias: Option<String>,
    owner: String,
    description: String,
    parents: Vec<String>,
    epoch: String,
    head: u64,
    buffer: VecDeque<(Record, usize)>,
    bytes: usize,
    writer: Option<u64>,
    notify: Arc<Notify>,
}
impl Stream {
    fn new(
        owner: String,
        alias: Option<String>,
        description: String,
        parents: Vec<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            alias,
            owner,
            description,
            parents,
            epoch: uuid::Uuid::new_v4().to_string(),
            head: 0,
            buffer: VecDeque::new(),
            bytes: 0,
            writer: None,
            notify: Arc::new(Notify::new()),
        }
    }
    fn oldest(&self) -> u64 {
        self.buffer
            .front()
            .map(|(r, _)| r.seq)
            .unwrap_or(self.head.saturating_add(1))
    }
    fn status(&self) -> Value {
        json!({"id":self.id,"alias":self.alias,"owner":self.owner,"description":self.description,"parents":self.parents,"epoch":self.epoch,"head":self.head,"oldest":self.oldest(),"buffer_records":self.buffer.len(),"buffer_bytes":self.bytes,"writer_active":self.writer.is_some()})
    }
    fn batch(&self, from: u64, limit: usize, wire_budget: usize) -> Result<Value> {
        let start = if from == 0 {
            self.oldest()
        } else {
            from.max(self.oldest())
        };
        if start > self.head.saturating_add(1) {
            return Err(fault("cursor_ahead", "cursor exceeds current stream head"));
        };
        let mut records = Vec::new();
        let mut bytes = 0;
        for (r, size) in &self.buffer {
            if r.seq < start {
                continue;
            };
            if records.len() >= limit || (!records.is_empty() && bytes + size > wire_budget) {
                break;
            };
            records.push(r.clone());
            bytes += size
        }
        let next = records.last().map(|r| r.seq + 1).unwrap_or(start);
        Ok(
            json!({"stream":self.id,"epoch":self.epoch,"from":start,"next":next,"head":self.head,"oldest":self.oldest(),"records":records}),
        )
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    stream: String,
    #[serde(default)]
    key: String,
    payload: Vec<u8>,
    #[serde(default)]
    source_ts_ns: Option<u64>,
    #[serde(default)]
    upstream: BTreeMap<String, u64>,
    #[serde(default)]
    source_seq: Option<u64>,
    #[serde(default)]
    channel: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    stream: String,
    #[serde(default)]
    limit: Option<usize>,
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
        let mut connections = self.core.connections.lock().unwrap();
        if connections
            .get(&self.plugin)
            .is_some_and(|c| c.generation == self.generation)
        {
            connections.remove(&self.plugin);
            for s in self.core.streams.read().unwrap().values() {
                let mut s = s.lock().unwrap();
                if s.owner == self.plugin && s.writer == Some(self.generation) {
                    s.writer = None
                }
            }
        }
    }
}
struct EventLease {
    core: Arc<Core>,
    plugin: String,
}
impl Drop for EventLease {
    fn drop(&mut self) {
        let mut events = self.core.event_connections.lock().unwrap();
        if let Some(count) = events.get_mut(&self.plugin) {
            *count -= 1;
            if *count == 0 {
                events.remove(&self.plugin);
            }
        }
    }
}
struct PendingCall {
    target: String,
    sender: oneshot::Sender<Result<Value, Fault>>,
}
pub struct Core {
    runtime: RuntimeConfig,
    streams: RwLock<BTreeMap<String, Arc<Mutex<Stream>>>>,
    aliases: RwLock<BTreeMap<String, String>>,
    attachments: RwLock<BTreeMap<String, Vec<String>>>,
    connections: Mutex<BTreeMap<String, ConnectionInfo>>,
    reports: Mutex<BTreeMap<String, Value>>,
    event_connections: Mutex<BTreeMap<String, usize>>,
    connection_budget: tokio::sync::Semaphore,
    calls: Mutex<BTreeMap<u64, PendingCall>>,
    sequence: AtomicU64,
}
impl Core {
    pub fn new(runtime: RuntimeConfig) -> Result<Arc<Self>> {
        validate(&runtime.config)?;
        if runtime.admin_token.is_empty() || runtime.plugin_tokens.values().any(|t| t.is_empty()) {
            bail!("authentication tokens cannot be empty")
        };
        let core = Arc::new(Self {
            runtime,
            streams: Default::default(),
            aliases: Default::default(),
            attachments: Default::default(),
            connections: Default::default(),
            reports: Default::default(),
            event_connections: Default::default(),
            connection_budget: tokio::sync::Semaphore::new(2048),
            calls: Default::default(),
            sequence: AtomicU64::new(1),
        });
        for p in &core.runtime.config.plugins {
            if let Some(spec) = p.streams.first() {
                let s = Stream::new(
                    p.id.clone(),
                    Some(spec.id.clone()),
                    spec.description.clone(),
                    spec.parents.clone(),
                );
                let id = s.id.clone();
                core.aliases
                    .write()
                    .unwrap()
                    .insert(spec.id.clone(), id.clone());
                core.streams
                    .write()
                    .unwrap()
                    .insert(id, Arc::new(Mutex::new(s)));
            }
        } // Convert configured parent aliases once, retaining only runtime identities in streams.
        for s in core.streams.read().unwrap().values() {
            let mut s = s.lock().unwrap();
            s.parents = s
                .parents
                .iter()
                .map(|p| {
                    core.aliases
                        .read()
                        .unwrap()
                        .get(p)
                        .cloned()
                        .unwrap_or_else(|| p.clone())
                })
                .collect()
        }
        Ok(core)
    }
    fn stream(&self, id: &str) -> Result<Arc<Mutex<Stream>>> {
        self.streams
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| fault("unknown_stream", id))
    }
    fn own(&self, plugin: &str) -> Option<Arc<Mutex<Stream>>> {
        self.streams
            .read()
            .unwrap()
            .values()
            .find(|s| s.lock().unwrap().owner == plugin)
            .cloned()
    }
    fn role(&self, plugin: &str) -> Result<Role> {
        self.runtime
            .config
            .plugins
            .iter()
            .find(|p| p.id == plugin)
            .map(|p| p.role)
            .ok_or_else(|| fault("permission_denied", "unknown plugin"))
    }
    fn can_read(&self, plugin: &str, id: &str) -> Result<()> {
        let s = self.stream(id)?;
        let s = s.lock().unwrap();
        if plugin == "__admin__" {
            return Ok(());
        };
        let p = self
            .runtime
            .config
            .plugins
            .iter()
            .find(|p| p.id == plugin)
            .ok_or_else(|| fault("permission_denied", "unknown plugin"))?;
        let reads = self
            .attachments
            .read()
            .unwrap()
            .get(plugin)
            .cloned()
            .unwrap_or_else(|| p.reads.clone());
        if p.role != Role::Output
            || s.owner == plugin
            || !reads.iter().any(|r| r == id || s.alias.as_ref() == Some(r))
        {
            return Err(fault(
                "permission_denied",
                "stream is not an authorized Output subscription",
            ));
        };
        Ok(())
    }
    fn read_streams(&self, plugin: &str) -> Vec<String> {
        let reads = self
            .attachments
            .read()
            .unwrap()
            .get(plugin)
            .cloned()
            .unwrap_or_else(|| {
                self.runtime
                    .config
                    .plugins
                    .iter()
                    .find(|p| p.id == plugin)
                    .map(|p| p.reads.clone())
                    .unwrap_or_default()
            });
        let aliases = self.aliases.read().unwrap();
        reads
            .into_iter()
            .map(|s| aliases.get(&s).cloned().unwrap_or(s))
            .collect()
    }
    fn prepare(&self, plugin: &str, args: Value) -> Result<Value> {
        if plugin != "__admin__" {
            return Err(fault(
                "permission_denied",
                "management authentication required",
            ));
        }
        let target = args["plugin"].as_str().context("plugin required")?;
        self.role(target)?;
        let connections = self.connections.lock().unwrap();
        let events = self.event_connections.lock().unwrap();
        if connections.contains_key(target) || events.get(target).copied().unwrap_or(0) != 0 {
            return Err(fault(
                "already_connected",
                "stop the plugin and close its subscriptions before preparing a new process",
            ));
        }
        self.reports.lock().unwrap().remove(target);
        Ok(json!({"plugin":target,"prepared":true}))
    }
    fn attach(&self, plugin: &str, args: Value) -> Result<Value> {
        if plugin != "__admin__" {
            return Err(fault(
                "permission_denied",
                "management authentication required",
            ));
        }
        let target = args["plugin"].as_str().context("plugin required")?;
        let id = args["stream"].as_str().context("stream required")?;
        if self.role(target)? != Role::Output {
            return Err(fault(
                "permission_denied",
                "only Output can attach to a readable stream",
            ));
        }
        let connections = self.connections.lock().unwrap();
        if connections.contains_key(target) {
            return Err(fault(
                "already_connected",
                "stop Output before changing its stream association",
            ));
        }
        let events = self.event_connections.lock().unwrap();
        if events.get(target).copied().unwrap_or(0) != 0 {
            return Err(fault(
                "already_connected",
                "close Output subscriptions before changing their association",
            ));
        }
        let stream = self.stream(id)?;
        if stream.lock().unwrap().owner == target {
            return Err(fault(
                "permission_denied",
                "Output cannot subscribe to its own derived stream",
            ));
        }
        // A configured derived stream has immutable parents for this Core lifetime.
        if let Some(own) = self.own(target) {
            let own = own.lock().unwrap();
            if !own.parents.is_empty() && own.parents != [id] {
                return Err(fault(
                    "invalid_parents",
                    "derived stream parent association cannot be rebound",
                ));
            }
        }
        // Follow actual process dependencies as well as explicit parent metadata.
        // Rebinding a consumer must never create a feedback path to its own output.
        let mut pending = vec![stream.lock().unwrap().owner.clone()];
        let mut seen = BTreeSet::new();
        while let Some(owner) = pending.pop() {
            if owner == target {
                return Err(fault(
                    "stream_cycle",
                    "association would create a stream feedback cycle",
                ));
            }
            if !seen.insert(owner.clone()) {
                continue;
            }
            for read in self.read_streams(&owner) {
                if let Ok(source) = self.stream(&read) {
                    pending.push(source.lock().unwrap().owner.clone());
                }
            }
        }
        self.attachments
            .write()
            .unwrap()
            .insert(target.into(), vec![id.into()]);
        Ok(json!({"plugin":target,"reads":[id]}))
    }
    fn create(
        &self,
        plugin: &str,
        generation: u64,
        description: String,
        parents: Vec<String>,
    ) -> Result<Value> {
        let role = self.role(plugin)?;
        if description.len() > 4096
            || parents.len() > 32
            || parents.iter().collect::<BTreeSet<_>>().len() != parents.len()
        {
            return Err(fault("limit", "description or parents exceeds limit"));
        };
        if role == Role::Input && !parents.is_empty() {
            return Err(fault("permission_denied", "Input cannot declare parents"));
        };
        for p in &parents {
            self.can_read(plugin, p)?
        }
        let mut streams = self.streams.write().unwrap();
        if let Some(s) = streams.values().find(|s| s.lock().unwrap().owner == plugin) {
            return Err(fault("stream_exists", s.lock().unwrap().id.clone()));
        };
        let mut s = Stream::new(plugin.into(), None, description, parents);
        s.writer = Some(generation);
        let value = s.status();
        streams.insert(s.id.clone(), Arc::new(Mutex::new(s)));
        Ok(value)
    }
    fn publish(&self, plugin: &str, generation: u64, args: Value) -> Result<Value> {
        let p: Publish = serde_json::from_value(args)?;
        let options = &self.runtime.config.core;
        if p.payload.len() > options.max_payload_bytes
            || p.key.len() > 256
            || p.channel.as_ref().is_some_and(|c| c.len() > 128)
        {
            return Err(fault("limit", "payload, key or channel exceeds limit"));
        };
        let stream = self.stream(&p.stream)?;
        let (parents, notify) = {
            let s = stream.lock().unwrap();
            if s.owner != plugin || s.writer != Some(generation) {
                return Err(fault(
                    "permission_denied",
                    "only the active stream writer may publish",
                ));
            };
            (s.parents.clone(), s.notify.clone())
        };
        if (parents.is_empty() && !p.upstream.is_empty())
            || (!parents.is_empty() && p.upstream.is_empty())
            || p.upstream.keys().any(|id| !parents.contains(id))
        {
            return Err(fault("invalid_parents", "derived records must identify at least one declared parent and may not invent parent streams"));
        }
        let mut epochs = BTreeMap::new();
        for (id, seq) in &p.upstream {
            let parent = self.stream(id)?;
            let parent = parent.lock().unwrap();
            if *seq == 0 || *seq > parent.head {
                return Err(fault("invalid_parent_cursor", id));
            };
            epochs.insert(id.clone(), parent.epoch.clone());
        }
        let mut s = stream.lock().unwrap();
        if s.owner != plugin || s.writer != Some(generation) {
            return Err(fault("permission_denied", "writer lease ended"));
        };
        let seq = s
            .head
            .checked_add(1)
            .ok_or_else(|| fault("limit", "stream sequence exhausted"))?;
        let record = Record {
            stream: s.id.clone(),
            epoch: s.epoch.clone(),
            seq,
            key: p.key,
            payload: p.payload,
            source_ts_ns: p.source_ts_ns,
            observed_ts_ns: now_ns(),
            upstream: p.upstream,
            upstream_epochs: epochs,
            source_seq: p.source_seq,
            channel: p.channel,
        };
        let size = serde_json::to_vec(&record)?.len();
        if size > options.buffer_bytes {
            return Err(fault(
                "limit",
                "encoded record exceeds per-stream byte buffer",
            ));
        };
        if options.transport == TransportKind::Udp && size > MAX_DATAGRAM - 1024 {
            return Err(fault("limit", "record exceeds UDP delivery budget"));
        };
        s.head = seq;
        s.bytes += size;
        s.buffer.push_back((record.clone(), size));
        while s.buffer.len() > options.buffer_records || s.bytes > options.buffer_bytes {
            if let Some((_, n)) = s.buffer.pop_front() {
                s.bytes -= n
            }
        }
        drop(s);
        notify.notify_waiters();
        Ok(serde_json::to_value(record)?)
    }
    fn read(&self, plugin: &str, args: Value) -> Result<Value> {
        let r: Read = serde_json::from_value(args)?;
        self.can_read(plugin, &r.stream)?;
        let limit = r
            .limit
            .unwrap_or(self.runtime.config.core.read_batch_records);
        if limit == 0 || limit > 64 {
            return Err(fault("limit", "read limit must be 1..64"));
        };
        let stream = self.stream(&r.stream)?;
        let budget = if self.runtime.config.core.transport == TransportKind::Udp {
            MAX_DATAGRAM - 1024
        } else {
            512 * 1024
        };
        let result = stream.lock().unwrap().batch(0, limit, budget);
        result
    }
    async fn command(
        self: &Arc<Self>,
        plugin: &str,
        generation: u64,
        op: &str,
        args: Value,
    ) -> Result<Value> {
        match op {
            "plugin.attach" => self.attach(plugin, args),
            "plugin.prepare" => self.prepare(plugin, args),
            "status" => {
                let states = self
                    .streams
                    .read()
                    .unwrap()
                    .values()
                    .map(|s| s.lock().unwrap().status())
                    .collect::<Vec<_>>();
                let connections = self.connections.lock().unwrap();
                let reports = self.reports.lock().unwrap();
                let plugins=self.runtime.config.plugins.iter().map(|p|json!({"id":p.id,"role":p.role,"connected":connections.contains_key(&p.id),"report":reports.get(&p.id)})).collect::<Vec<_>>();
                Ok(
                    json!({"pid":std::process::id(),"protocol":PROTOCOL,"streams":states,"plugins":plugins,"config":self.runtime.config,"effective_core":self.runtime.config.core}),
                )
            }
            "streams" => Ok(Value::Array(
                self.streams
                    .read()
                    .unwrap()
                    .values()
                    .map(|s| s.lock().unwrap().status())
                    .collect(),
            )),
            "stream.get" => {
                let id = args["stream"].as_str().context("stream required")?;
                Ok(self.stream(id)?.lock().unwrap().status())
            }
            "stream.resolve" => {
                let alias = args["alias"].as_str().context("alias required")?;
                let id = self
                    .aliases
                    .read()
                    .unwrap()
                    .get(alias)
                    .cloned()
                    .unwrap_or_else(|| alias.into());
                Ok(self.stream(&id)?.lock().unwrap().status())
            }
            "stream.own" => Ok(self
                .own(plugin)
                .map(|s| s.lock().unwrap().status())
                .unwrap_or(Value::Null)),
            "stream.create" => {
                let description = args
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let parents =
                    serde_json::from_value(args.get("parents").cloned().unwrap_or(json!([])))?;
                self.create(plugin, generation, description, parents)
            }
            "stream.claim" => {
                let id = args["stream"].as_str().context("stream required")?;
                let stream = self.stream(id)?;
                let mut s = stream.lock().unwrap();
                if s.owner != plugin {
                    return Err(fault(
                        "permission_denied",
                        "knowing a stream ID does not grant writing",
                    ));
                };
                if s.writer.is_some_and(|g| g != generation) {
                    return Err(fault("writer_occupied", "stream already has a writer"));
                };
                s.writer = Some(generation);
                Ok(s.status())
            }
            "stream.describe" => {
                let id = args["stream"].as_str().context("stream required")?;
                let description = args["description"]
                    .as_str()
                    .context("description required")?;
                if description.len() > 4096 {
                    return Err(fault("limit", "description exceeds 4096 bytes"));
                };
                let stream = self.stream(id)?;
                let mut s = stream.lock().unwrap();
                if plugin != "__admin__" && (s.owner != plugin || s.writer != Some(generation)) {
                    return Err(fault(
                        "permission_denied",
                        "owner or management authentication required",
                    ));
                };
                s.description = description.into();
                Ok(s.status())
            }
            "read" => self.read(plugin, args),
            "publish" => self.publish(plugin, generation, args),
            "config.patch" => Err(fault(
                "restart_required",
                "configuration is a startup snapshot; restart the main process to apply changes",
            )),
            "report" => {
                if serde_json::to_vec(&args)?.len() > 16384 {
                    return Err(fault("limit", "report too large"));
                };
                self.reports.lock().unwrap().insert(plugin.into(), args);
                Ok(json!({"accepted":true}))
            }
            "control" => {
                if plugin != "__admin__" {
                    return Err(fault(
                        "permission_denied",
                        "management authentication required",
                    ));
                };
                let target = args["target"]
                    .as_str()
                    .context("target required")?
                    .to_string();
                let method = args["method"]
                    .as_str()
                    .context("method required")?
                    .to_string();
                let id = self.sequence.fetch_add(1, Ordering::Relaxed);
                let (tx, rx) = oneshot::channel();
                {
                    let connections = self.connections.lock().unwrap();
                    let connection = connections
                        .get(&target)
                        .ok_or_else(|| fault("not_connected", &target))?;
                    let mut calls = self.calls.lock().unwrap();
                    if calls.len() >= 128 {
                        return Err(fault("busy", "too many controls"));
                    };
                    calls.insert(
                        id,
                        PendingCall {
                            target: target.clone(),
                            sender: tx,
                        },
                    );
                    if connection
                        .sender
                        .try_send(ServerMessage::Control {
                            call_id: id,
                            method,
                            args: args.get("args").cloned().unwrap_or(json!({})),
                        })
                        .is_err()
                    {
                        calls.remove(&id);
                        return Err(fault("control_busy", target));
                    }
                }
                let result = tokio::time::timeout(Duration::from_secs(10), rx).await;
                self.calls.lock().unwrap().remove(&id);
                match result {
                    Ok(Ok(v)) => v.map_err(Into::into),
                    _ => Err(fault(
                        "control_timeout",
                        "plugin did not confirm within 10s; outcome unknown",
                    )),
                }
            }
            "reply" => {
                let id = args["call_id"].as_u64().context("call_id required")?;
                let mut calls = self.calls.lock().unwrap();
                if calls.get(&id).is_some_and(|c| c.target != plugin) {
                    return Err(fault("permission_denied", "control reply owner mismatch"));
                };
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
        conn: &mut ServerConnection,
    ) -> Result<()> {
        let request = conn
            .receive()
            .await
            .context("subscription request missing")?;
        let setup = (|| -> Result<_> {
            if request.op != "subscribe" {
                return Err(fault(
                    "invalid_request",
                    "event connection requires subscribe",
                ));
            };
            let r: Read = serde_json::from_value(request.args)?;
            self.can_read(plugin, &r.stream)?;
            if r.limit.is_some_and(|n| n == 0 || n > 64) {
                return Err(fault("limit", "subscription limit must be 1..64"));
            };
            let stream = self.stream(&r.stream)?;
            let s = stream.lock().unwrap();
            let next = s.oldest();
            let welcome =
                json!({"stream":s.id,"epoch":s.epoch,"from":next,"head":s.head,"subscribed":true});
            Ok((stream.clone(), s.notify.clone(), next, welcome))
        })();
        let (stream, notify, mut next, welcome) = match setup {
            Ok(s) => s,
            Err(e) => {
                conn.send(ServerMessage::Response {
                    id: request.id,
                    result: Value::Null,
                    error: Some(wire_error(e)),
                })
                .await?;
                return Ok(());
            }
        };
        conn.send(ServerMessage::Response {
            id: request.id,
            result: welcome,
            error: None,
        })
        .await?;
        loop {
            if !matches!(conn.try_receive(), Ok(None)) {
                return Ok(());
            }
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let records = {
                let s = stream.lock().unwrap();
                next = next.max(s.oldest());
                s.buffer
                    .iter()
                    .filter(|(r, _)| r.seq >= next)
                    .take(self.runtime.config.core.read_batch_records)
                    .map(|(r, _)| r.clone())
                    .collect::<Vec<_>>()
            };
            if records.is_empty() {
                tokio::select! {_=&mut notified=>{},_ = conn.receive()=>return Ok(())}
            } else {
                for record in records {
                    next = record.seq + 1;
                    conn.send(ServerMessage::Record { record }).await?
                }
            }
        }
    }
    pub async fn connection(self: Arc<Self>, stream: tokio::net::TcpStream) -> Result<()> {
        self.serve_connection(ServerConnection::tcp(stream).await?)
            .await
    }
    pub async fn serve_connection(self: Arc<Self>, mut conn: ServerConnection) -> Result<()> {
        let _connection_budget = match self.connection_budget.try_acquire() {
            Ok(permit) => permit,
            Err(_) => {
                conn.send(ServerMessage::Response {
                    id: 0,
                    result: Value::Null,
                    error: Some(Fault {
                        code: "busy".into(),
                        message: "Core connection resource budget exhausted".into(),
                    }),
                })
                .await?;
                return Ok(());
            }
        };
        let hello = conn.hello.clone();
        let authenticated = (|| -> Result<()> {
            if hello.protocol != PROTOCOL {
                return Err(fault("version_mismatch", format!("requires {PROTOCOL}")));
            };
            if hello.plugin == "__admin__" {
                if hello.token != self.runtime.admin_token {
                    return Err(fault("authentication_failed", "invalid management token"));
                }
            } else {
                if self
                    .runtime
                    .plugin_tokens
                    .get(&hello.plugin)
                    .is_none_or(|t| *t != hello.token)
                {
                    return Err(fault(
                        "authentication_failed",
                        "invalid plugin identity/token",
                    ));
                };
                self.role(&hello.plugin)?;
            };
            Ok(())
        })();
        if let Err(e) = authenticated {
            conn.send(ServerMessage::Response {
                id: 0,
                result: Value::Null,
                error: Some(wire_error(e)),
            })
            .await?;
            return Ok(());
        };
        let generation = self.sequence.fetch_add(1, Ordering::Relaxed);
        let (tx, mut rx) = mpsc::channel(self.runtime.config.core.queue_records);
        let mut lease = None;
        if !hello.events && hello.plugin != "__admin__" {
            let setup = (|| -> Result<()> {
                let mut connections = self.connections.lock().unwrap();
                if connections.contains_key(&hello.plugin) {
                    return Err(fault(
                        "writer_occupied",
                        "plugin already has an active connection",
                    ));
                };
                connections.insert(
                    hello.plugin.clone(),
                    ConnectionInfo {
                        generation,
                        sender: tx.clone(),
                    },
                );
                // Reports describe an individual running plugin generation.
                self.reports.lock().unwrap().remove(&hello.plugin);
                Ok(())
            })();
            if let Err(e) = setup {
                conn.send(ServerMessage::Response {
                    id: 0,
                    result: Value::Null,
                    error: Some(wire_error(e)),
                })
                .await?;
                return Ok(());
            };
            lease = Some(ConnectionLease {
                core: self.clone(),
                plugin: hello.plugin.clone(),
                generation,
            });
            if let Some(s) = self.own(&hello.plugin) {
                s.lock().unwrap().writer = Some(generation)
            } else if self.role(&hello.plugin)? == Role::Input {
                self.create(&hello.plugin, generation, hello.plugin.clone(), vec![])?;
            }
        }
        let _event_lease = if hello.events {
            *self
                .event_connections
                .lock()
                .unwrap()
                .entry(hello.plugin.clone())
                .or_default() += 1;
            Some(EventLease {
                core: self.clone(),
                plugin: hello.plugin.clone(),
            })
        } else {
            None
        };
        let owned = self.own(&hello.plugin).map(|s| s.lock().unwrap().status());
        conn.send(ServerMessage::Response {
            id: 0,
            result: json!({"protocol":PROTOCOL,"plugin":hello.plugin,"stream":owned,"reads":self.read_streams(&hello.plugin)}),
            error: None,
        })
        .await?;
        if hello.events {
            return self.subscription(&hello.plugin, &mut conn).await;
        };
        loop {
            tokio::select! {message=rx.recv()=>{if let Some(message)=message {conn.send(message).await?}},request=conn.receive()=>{let Some(request)=request else{break};if request.op=="disconnect" { break; } let no_ack=conn.transport==TransportKind::Udp&&request.op=="publish";let result=self.command(&hello.plugin,generation,&request.op,request.args).await;if !no_ack {let(result,error)=match result {Ok(v)=>(v,None),Err(e)=>(Value::Null,Some(wire_error(e)))};let mut response = ServerMessage::Response{id:request.id,result,error};
            let max = if conn.transport == TransportKind::Udp { MAX_DATAGRAM } else { log_proto::MAX_WIRE - 1 };
            if serde_json::to_vec(&response)?.len() > max {
                response = ServerMessage::Response{id:request.id,result:Value::Null,error:Some(Fault{code:"limit".into(),message:"response exceeds transport frame budget; reduce the requested batch".into()})};
            }
            conn.send(response).await?;}}}
        }
        drop(lease);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log_proto::{
        ClientConnection, ClientReader, ClientWriter, CoreOptions, Hello, PluginSpec, Request,
        ServerListener, StreamSpec,
    };
    fn spec(id: &str, role: Role, alias: Option<&str>, reads: &[&str]) -> PluginSpec {
        PluginSpec {
            id: id.into(),
            role,
            bin: "test".into(),
            args: vec![],
            autostart: false,
            reads: reads.iter().map(|s| s.to_string()).collect(),
            streams: alias
                .into_iter()
                .map(|alias| StreamSpec {
                    id: alias.into(),
                    description: format!("description {id}"),
                    parents: vec![],
                })
                .collect(),
            config: json!({}),
        }
    }
    fn runtime(transport: TransportKind) -> RuntimeConfig {
        let plugins = vec![
            spec("a", Role::Input, Some("alpha"), &[]),
            spec("b", Role::Input, Some("beta"), &[]),
            spec("out", Role::Output, None, &["alpha"]),
            spec("other", Role::Output, None, &["alpha", "beta"]),
            spec("derive", Role::Output, None, &["alpha"]),
        ];
        RuntimeConfig {
            plugin_tokens: plugins
                .iter()
                .map(|p| (p.id.clone(), format!("{}-token", p.id)))
                .collect(),
            config: Config {
                core: CoreOptions {
                    transport,
                    ..CoreOptions::default()
                },
                plugins,
            },
            admin_token: "admin-token".into(),
        }
    }
    fn claim(core: &Core, plugin: &str, generation: u64) -> String {
        let s = core.own(plugin).unwrap();
        let mut s = s.lock().unwrap();
        s.writer = Some(generation);
        s.id.clone()
    }
    fn publish(
        core: &Core,
        plugin: &str,
        generation: u64,
        id: &str,
        source_seq: u64,
        payload: &[u8],
    ) -> Result<Value> {
        core.publish(
            plugin,
            generation,
            json!({"stream":id,"key":"same opaque key","source_seq":source_seq,"payload":payload}),
        )
    }
    #[test]
    fn uuid_identity_and_receive_order_are_independent_of_alias_source_sequence_and_key() {
        let core = Core::new(runtime(TransportKind::Tcp)).unwrap();
        let id = claim(&core, "a", 1);
        assert_ne!(id, "alpha");
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        for seq in [9, 1, 9] {
            publish(&core, "a", 1, &id, seq, b"x").unwrap();
        }
        let result = core.read("out", json!({"stream":id})).unwrap();
        let records = result["records"].as_array().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|r| r["source_seq"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [9, 1, 9]
        );
        assert_eq!(
            records
                .iter()
                .map(|r| r["seq"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(records[0].get("durability").is_none());
        assert!(core.stream("alpha").is_err());
        let restarted = Core::new(runtime(TransportKind::Tcp)).unwrap();
        let fresh = claim(&restarted, "a", 1);
        assert_ne!(id, fresh);
        assert_eq!(restarted.stream(&fresh).unwrap().lock().unwrap().head, 0);
    }
    #[test]
    fn record_and_byte_caps_roll_independently_and_isolate_streams() {
        let mut r = runtime(TransportKind::Tcp);
        r.config.core.buffer_records = 2;
        r.config.core.buffer_bytes = 1500;
        let core = Core::new(r).unwrap();
        let a = claim(&core, "a", 1);
        let b = claim(&core, "b", 2);
        publish(&core, "b", 2, &b, 1, b"untouched").unwrap();
        for seq in 1..=4 {
            publish(&core, "a", 1, &a, seq, b"one").unwrap();
        }
        let s = core.stream(&a).unwrap();
        assert_eq!(s.lock().unwrap().oldest(), 3);
        publish(&core, "a", 1, &a, 5, &vec![255; 280]).unwrap();
        let s = s.lock().unwrap();
        assert!(s.bytes <= 1500);
        assert_eq!(
            s.buffer.len(),
            1,
            "serialized metadata and expanded byte payload both count against the byte cap"
        );
        assert_eq!(s.oldest(), 5);
        assert_eq!(core.stream(&b).unwrap().lock().unwrap().buffer.len(), 1);
    }
    #[test]
    fn oversized_record_rejection_is_atomic_and_foreign_writers_are_rejected() {
        let mut r = runtime(TransportKind::Tcp);
        r.config.core.buffer_bytes = 500;
        let core = Core::new(r).unwrap();
        let id = claim(&core, "a", 8);
        assert!(publish(&core, "b", 8, &id, 1, b"bad").is_err());
        assert!(publish(&core, "a", 7, &id, 1, b"stale generation").is_err());
        assert!(publish(&core, "a", 8, &id, 1, &vec![255; 300]).is_err());
        assert_eq!(core.stream(&id).unwrap().lock().unwrap().head, 0);
        assert!(core.can_read("a", &id).is_err());
        let beta = claim(&core, "b", 2);
        assert!(core.can_read("out", &beta).is_err());
    }
    #[tokio::test]
    async fn description_does_not_change_identity_or_authorization_and_static_options_reject_patch()
    {
        let core = Core::new(runtime(TransportKind::Tcp)).unwrap();
        let id = claim(&core, "a", 1);
        let updated = core
            .command(
                "a",
                1,
                "stream.describe",
                json!({"stream":id,"description":"中文备注"}),
            )
            .await
            .unwrap();
        assert_eq!(updated["id"], id);
        assert_eq!(updated["description"], "中文备注");
        assert!(core
            .command(
                "b",
                2,
                "stream.describe",
                json!({"stream":id,"description":"intruder"})
            )
            .await
            .is_err());
        assert!(core
            .command("__admin__", 0, "config.patch", json!({"buffer_records":1}))
            .await
            .is_err());
        assert!(core.create("a", 1, "second".into(), vec![]).is_err());
        assert!(core
            .command("b", 2, "stream.claim", json!({"stream":id}))
            .await
            .is_err());
    }
    #[test]
    fn derived_stream_requires_authorized_parents_and_never_writes_original() {
        let core = Core::new(runtime(TransportKind::Tcp)).unwrap();
        let id = claim(&core, "a", 1);
        publish(&core, "a", 1, &id, 1, b"raw").unwrap();
        let created = core
            .create("derive", 3, "converted".into(), vec![id.clone()])
            .unwrap();
        let derived = created["id"].as_str().unwrap();
        assert_ne!(derived, id);
        assert!(core
            .publish(
                "derive",
                3,
                json!({"stream":derived,"payload":[1],"upstream":{}})
            )
            .is_err());
        let result = core
            .publish(
                "derive",
                3,
                json!({"stream":derived,"payload":[1],"upstream":{id.clone():1}}),
            )
            .unwrap();
        assert_eq!(result["upstream"][&id], 1);
        assert_eq!(core.stream(&id).unwrap().lock().unwrap().head, 1);
        assert!(core.can_read("derive", derived).is_err());
    }
    struct TestServer {
        address: String,
        core: Arc<Core>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn server(transport: TransportKind, records: usize) -> TestServer {
        let mut r = runtime(transport);
        r.config.core.buffer_records = records;
        let core = Core::new(r).unwrap();
        let mut listener = ServerListener::bind("127.0.0.1:0", transport)
            .await
            .unwrap();
        let address = listener.local_addr().to_string();
        let serving = core.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {conn=listener.accept()=>{match conn {Ok(conn)=>{let core=serving.clone();tasks.spawn(async move {let _=core.serve_connection(conn).await;});},Err(_)=>break}},Some(_)=tasks.join_next()=>{}}
            }
        });
        TestServer {
            address,
            core,
            task,
        }
    }
    struct Peer {
        r: ClientReader,
        w: ClientWriter,
        next: u64,
        welcome: Value,
    }
    impl Drop for Peer {
        fn drop(&mut self) {
            self.w.disconnect();
        }
    }
    impl Peer {
        async fn new(
            server: &TestServer,
            transport: TransportKind,
            plugin: &str,
            events: bool,
        ) -> Self {
            let token = if plugin == "__admin__" {
                "admin-token".into()
            } else {
                format!("{plugin}-token")
            };
            let (r, w) = ClientConnection::connect(
                &server.address,
                transport,
                &Hello {
                    protocol: PROTOCOL.into(),
                    plugin: plugin.into(),
                    token,
                    events,
                },
            )
            .await
            .unwrap();
            let mut peer = Self {
                r,
                w,
                next: 1,
                welcome: Value::Null,
            };
            peer.welcome = peer.response().await.unwrap();
            peer
        }
        async fn message(&mut self) -> ServerMessage {
            tokio::time::timeout(Duration::from_secs(3), self.r.receive())
                .await
                .expect("timed out")
                .unwrap()
                .expect("closed")
        }
        async fn response(&mut self) -> Result<Value> {
            match self.message().await {
                ServerMessage::Response {
                    result,
                    error: None,
                    ..
                } => Ok(result),
                ServerMessage::Response { error: Some(e), .. } => Err(e.into()),
                other => panic!("expected response, got {other:?}"),
            }
        }
        async fn call(&mut self, op: &str, args: Value) -> Result<Value> {
            let id = self.next;
            self.next += 1;
            self.w
                .send(&Request {
                    id,
                    op: op.into(),
                    args,
                })
                .await
                .unwrap();
            self.response().await
        }
        async fn record(&mut self) -> Record {
            match self.message().await {
                ServerMessage::Record { record } => record,
                other => panic!("expected record, got {other:?}"),
            }
        }
    }
    #[tokio::test]
    async fn tcp_two_outputs_replay_retained_records_then_wait_and_receive_independently() {
        let server = server(TransportKind::Tcp, 2).await;
        let mut input = Peer::new(&server, TransportKind::Tcp, "a", false).await;
        let id = input.welcome["stream"]["id"].as_str().unwrap().to_owned();
        for n in 1..=3 {
            input
                .call("publish", json!({"stream":id,"payload":[n]}))
                .await
                .unwrap();
        }
        let mut out = Peer::new(&server, TransportKind::Tcp, "out", true).await;
        let mut other = Peer::new(&server, TransportKind::Tcp, "other", true).await;
        for peer in [&mut out, &mut other] {
            let welcome = peer.call("subscribe", json!({"stream":id})).await.unwrap();
            assert_eq!(welcome["from"], 2);
            assert_eq!(peer.record().await.seq, 2);
            assert_eq!(peer.record().await.seq, 3);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(40), out.r.receive())
                .await
                .is_err()
        );
        input
            .call("publish", json!({"stream":id,"payload":[4]}))
            .await
            .unwrap();
        assert_eq!(out.record().await.seq, 4);
        assert_eq!(other.record().await.seq, 4);
        drop(input);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let s = server.core.stream(&id).unwrap();
        assert_eq!(s.lock().unwrap().head, 4);
        assert!(s.lock().unwrap().writer.is_none());
    }
    #[tokio::test]
    async fn empty_subscription_waits_without_eof_and_foreign_second_writer_is_rejected() {
        let server = server(TransportKind::Tcp, 16).await;
        let mut input = Peer::new(&server, TransportKind::Tcp, "a", false).await;
        let id = input.welcome["stream"]["id"].as_str().unwrap().to_owned();
        let mut out = Peer::new(&server, TransportKind::Tcp, "out", true).await;
        out.call("subscribe", json!({"stream":id})).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(40), out.r.receive())
                .await
                .is_err()
        );
        let (mut reader, _writer) = ClientConnection::connect(
            &server.address,
            TransportKind::Tcp,
            &Hello {
                protocol: PROTOCOL.into(),
                plugin: "a".into(),
                token: "a-token".into(),
                events: false,
            },
        )
        .await
        .unwrap();
        match reader.receive().await.unwrap().unwrap() {
            ServerMessage::Response { error: Some(e), .. } => assert_eq!(e.code, "writer_occupied"),
            _ => panic!("second writer accepted"),
        };
        input
            .call("publish", json!({"stream":id,"payload":[1]}))
            .await
            .unwrap();
        assert_eq!(out.record().await.seq, 1);
    }
    #[tokio::test]
    async fn udp_handshake_confirmed_publish_has_no_ack_and_disconnect_releases_writer() {
        let server = server(TransportKind::Udp, 16).await;
        let mut input = Peer::new(&server, TransportKind::Udp, "a", false).await;
        let id = input.welcome["stream"]["id"].as_str().unwrap().to_owned();
        let mut out = Peer::new(&server, TransportKind::Udp, "out", true).await;
        out.call("subscribe", json!({"stream":id})).await.unwrap();
        input
            .w
            .send(&Request {
                id: 42,
                op: "publish".into(),
                args: json!({"stream":id,"payload":[1],"source_seq":7}),
            })
            .await
            .unwrap();
        assert_eq!(out.record().await.source_seq, Some(7));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), input.r.receive())
                .await
                .is_err(),
            "UDP publication must not produce a per-record acknowledgment"
        );
        input.w.disconnect();
        tokio::time::timeout(Duration::from_secs(2), async {
            while server
                .core
                .stream(&id)
                .unwrap()
                .lock()
                .unwrap()
                .writer
                .is_some()
            {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        let again = Peer::new(&server, TransportKind::Udp, "a", false).await;
        assert_eq!(again.welcome["stream"]["id"], id);
        assert_eq!(again.welcome["stream"]["head"], 1);
    }
    #[tokio::test]
    async fn binding_only_offline_outputs_returns_actual_ids_and_preserves_input_ownership() {
        let server = server(TransportKind::Tcp, 16).await;
        let mut admin = Peer::new(&server, TransportKind::Tcp, "__admin__", false).await;
        let b = server.core.own("b").unwrap().lock().unwrap().id.clone();
        let a = server.core.own("a").unwrap().lock().unwrap().id.clone();
        admin
            .call("plugin.attach", json!({"plugin":"out","stream":b}))
            .await
            .unwrap();
        let _out = Peer::new(&server, TransportKind::Tcp, "out", false).await;
        assert_eq!(_out.welcome["reads"], json!([b]));
        assert!(server.core.can_read("out", &a).is_err());
        assert!(server.core.can_read("out", &b).is_ok());
        assert!(admin
            .call("plugin.attach", json!({"plugin":"out","stream":a}))
            .await
            .is_err());
        assert!(admin
            .call("plugin.attach", json!({"plugin":"a","stream":b}))
            .await
            .is_err());
    }
    #[tokio::test]
    async fn wrong_token_and_v1_protocol_never_register() {
        let server = server(TransportKind::Tcp, 16).await;
        for (protocol, token, code) in [
            (PROTOCOL, "wrong", "authentication_failed"),
            ("log-print/1", "a-token", "version_mismatch"),
        ] {
            let (mut r, _w) = ClientConnection::connect(
                &server.address,
                TransportKind::Tcp,
                &Hello {
                    protocol: protocol.into(),
                    plugin: "a".into(),
                    token: token.into(),
                    events: false,
                },
            )
            .await
            .unwrap();
            match r.receive().await.unwrap().unwrap() {
                ServerMessage::Response { error: Some(e), .. } => assert_eq!(e.code, code),
                _ => panic!("invalid authentication accepted"),
            }
        }
        assert!(server.core.connections.lock().unwrap().is_empty());
    }
    #[test]
    fn config_rejects_old_save_fields_multiple_input_streams_and_feedback_cycles() {
        assert!(
            serde_json::from_value::<Config>(json!({"core":{"save":{"enabled":true}}})).is_err()
        );
        let mut r = runtime(TransportKind::Tcp);
        r.config.plugins[0].streams.push(StreamSpec {
            id: "extra".into(),
            description: String::new(),
            parents: vec![],
        });
        assert!(validate(&r.config).is_err());
        let mut r = runtime(TransportKind::Tcp);
        r.config.plugins[0].role = Role::Output;
        r.config.plugins[0].reads = vec!["beta".into()];
        r.config.plugins[1].role = Role::Output;
        r.config.plugins[1].reads = vec!["alpha".into()];
        assert!(validate(&r.config).is_err());
    }
    #[tokio::test]
    async fn stalled_output_does_not_hold_input_or_another_stream() {
        let server = server(TransportKind::Tcp, 2).await;
        let mut input = Peer::new(&server, TransportKind::Tcp, "a", false).await;
        let mut other_input = Peer::new(&server, TransportKind::Tcp, "b", false).await;
        let id = input.welcome["stream"]["id"].as_str().unwrap().to_owned();
        let other_id = other_input.welcome["stream"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut stalled = Peer::new(&server, TransportKind::Tcp, "out", true).await;
        stalled
            .call("subscribe", json!({"stream":id}))
            .await
            .unwrap();
        // Leave the subscriber socket unread while enough encoded data exceeds
        // normal socket buffers. The producer and other stream must still answer.
        let generation = server.core.connections.lock().unwrap()["a"].generation;
        for n in 0..512 {
            publish(&server.core, "a", generation, &id, n, &vec![255; 16 * 1024]).unwrap();
            if n % 2 == 0 {
                tokio::task::yield_now().await;
            }
        }
        let accepted = tokio::time::timeout(
            Duration::from_secs(2),
            input.call("publish", json!({"stream":id,"payload":[1]})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(accepted["seq"], 513);
        let isolated = tokio::time::timeout(
            Duration::from_secs(2),
            other_input.call("publish", json!({"stream":other_id,"payload":[2]})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(isolated["seq"], 1);
        assert_eq!(
            server
                .core
                .stream(&id)
                .unwrap()
                .lock()
                .unwrap()
                .buffer
                .len(),
            2
        );
        assert_eq!(
            server
                .core
                .stream(&other_id)
                .unwrap()
                .lock()
                .unwrap()
                .buffer
                .len(),
            1
        );
    }
    #[tokio::test]
    async fn reconnect_clears_previous_generation_report_without_interpreting_plugin_states() {
        let server = server(TransportKind::Tcp, 16).await;
        let mut first = Peer::new(&server, TransportKind::Tcp, "a", false).await;
        first
            .call("report", json!({"state":"capturing","generation":"old"}))
            .await
            .unwrap();
        drop(first);
        tokio::time::timeout(Duration::from_secs(2), async {
            while server.core.connections.lock().unwrap().contains_key("a") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(server.core.reports.lock().unwrap().contains_key("a"));
        let mut next = Peer::new(&server, TransportKind::Tcp, "a", false).await;
        let status = server
            .core
            .command("__admin__", 0, "status", json!({}))
            .await
            .unwrap();
        let a = status["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "a")
            .unwrap();
        assert_eq!(a["connected"], true);
        assert!(a["report"].is_null());
        assert!(a.get("ready").is_none());
        next.call("report", json!({"state":"following","generation":"new"}))
            .await
            .unwrap();
        assert_eq!(
            server.core.reports.lock().unwrap()["a"]["generation"],
            "new"
        );
    }
    #[tokio::test]
    async fn live_event_connections_block_rebinding_until_explicit_disconnect_for_tcp_and_udp() {
        for transport in [TransportKind::Tcp, TransportKind::Udp] {
            let server = server(transport, 16).await;
            let a = server.core.own("a").unwrap().lock().unwrap().id.clone();
            let b = server.core.own("b").unwrap().lock().unwrap().id.clone();
            let mut out = Peer::new(&server, transport, "out", true).await;
            out.call("subscribe", json!({"stream":a})).await.unwrap();
            assert!(server
                .core
                .attach("__admin__", json!({"plugin":"out","stream":b}))
                .is_err());
            drop(out);
            tokio::time::timeout(Duration::from_secs(2), async {
                while server
                    .core
                    .event_connections
                    .lock()
                    .unwrap()
                    .contains_key("out")
                {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(server
                .core
                .attach("__admin__", json!({"plugin":"out","stream":b}))
                .is_ok());
        }
    }
    #[test]
    fn runtime_association_detects_process_feedback_even_without_declared_parent_metadata() {
        let mut r = runtime(TransportKind::Tcp);
        r.config.plugins.push(spec(
            "first",
            Role::Output,
            Some("first-stream"),
            &["alpha"],
        ));
        r.config.plugins.push(spec(
            "second",
            Role::Output,
            Some("second-stream"),
            &["first-stream"],
        ));
        for id in ["first", "second"] {
            r.plugin_tokens.insert(id.into(), format!("{id}-token"));
        }
        let core = Core::new(r).unwrap();
        let second = core.own("second").unwrap().lock().unwrap().id.clone();
        let err = core
            .attach("__admin__", json!({"plugin":"first","stream":second}))
            .unwrap_err();
        assert_eq!(err.downcast_ref::<Fault>().unwrap().code, "stream_cycle");
        assert_eq!(
            core.read_streams("first"),
            vec![core.own("a").unwrap().lock().unwrap().id.clone()]
        );
    }
    #[test]
    fn multisource_derived_records_reference_only_their_actual_declared_source() {
        let core = Core::new(runtime(TransportKind::Tcp)).unwrap();
        let a = claim(&core, "a", 1);
        let b = claim(&core, "b", 2);
        publish(&core, "a", 1, &a, 1, b"a").unwrap();
        publish(&core, "b", 2, &b, 1, b"b").unwrap();
        let output = core
            .create("other", 3, "two sources".into(), vec![a.clone(), b.clone()])
            .unwrap();
        let id = output["id"].as_str().unwrap();
        for source in [&a, &b] {
            let record = core
                .publish(
                    "other",
                    3,
                    json!({"stream":id,"payload":[1],"upstream":{source:1}}),
                )
                .unwrap();
            assert_eq!(record["upstream"].as_object().unwrap().len(), 1);
        }
        assert_eq!(core.stream(id).unwrap().lock().unwrap().head, 2);
        assert!(core
            .publish(
                "other",
                3,
                json!({"stream":id,"payload":[1],"upstream":{"foreign":1}})
            )
            .is_err());
    }
    #[tokio::test]
    async fn udp_short_management_sessions_do_not_exhaust_the_listener_session_budget() {
        let server = server(TransportKind::Udp, 16).await;
        for _ in 0..1030 {
            let mut admin = Peer::new(&server, TransportKind::Udp, "__admin__", false).await;
            admin.call("streams", json!({})).await.unwrap();
            // Drop sends one best-effort disconnect; no retries or idle reclamation.
        }
        let input = Peer::new(&server, TransportKind::Udp, "a", false).await;
        assert!(input.welcome["stream"]["id"].is_string());
    }
    #[tokio::test]
    async fn prepare_requires_management_and_offline_plugin_and_preserves_stream_data() {
        let server = server(TransportKind::Tcp, 16).await;
        let mut input = Peer::new(&server, TransportKind::Tcp, "a", false).await;
        let id = input.welcome["stream"]["id"].as_str().unwrap().to_owned();
        input
            .call("publish", json!({"stream":id,"payload":[1,2,3]}))
            .await
            .unwrap();
        input
            .call("report", json!({"state":"source_eof"}))
            .await
            .unwrap();
        assert!(server
            .core
            .prepare("__admin__", json!({"plugin":"a"}))
            .is_err());
        assert!(server.core.prepare("b", json!({"plugin":"a"})).is_err());
        drop(input);
        tokio::time::timeout(Duration::from_secs(2), async {
            while server.core.connections.lock().unwrap().contains_key("a") {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let before = server.core.stream(&id).unwrap().lock().unwrap().status();
        let reads = server.core.read_streams("out");
        let result = server
            .core
            .command("__admin__", 0, "plugin.prepare", json!({"plugin":"a"}))
            .await
            .unwrap();
        assert_eq!(result["prepared"], true);
        assert!(!server.core.reports.lock().unwrap().contains_key("a"));
        assert_eq!(
            server.core.stream(&id).unwrap().lock().unwrap().status(),
            before
        );
        assert_eq!(server.core.read_streams("out"), reads);
        let mut out = Peer::new(&server, TransportKind::Tcp, "out", true).await;
        out.call("subscribe", json!({"stream":id})).await.unwrap();
        assert!(server
            .core
            .prepare("__admin__", json!({"plugin":"out"}))
            .is_err());
        assert!(server
            .core
            .prepare("__admin__", json!({"plugin":"unknown"}))
            .is_err());
    }
}
