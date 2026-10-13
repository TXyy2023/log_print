//! Output-file owns serialization, checkpoints and durable commits. This module
//! coordinates a fixed historical prefix with a bounded, continuously read tail.
use super::*;
use output_file::{Archive, Config, Cursor, Gap};
use std::time::Duration;
use tokio::io::BufReader as AsyncBufReader;

pub(crate) struct Recording {
    pub status: Mutex<Value>,
    pub stop: AtomicBool,
    pub done: tokio::sync::Notify,
    pub path: PathBuf,
}
impl Engine {
    pub(crate) fn working_dir(&self, name: &str) -> Result<PathBuf> {
        let path = PathBuf::from(
            self.config["state_path"]
                .as_str()
                .context("state_path required")?,
        )
        .parent()
        .context("state directory")?
        .join(name);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }
    pub(super) fn history_path(&self, args: &Value) -> Option<PathBuf> {
        let imports = self.imports.lock().unwrap();
        if let Some(id) = args["streams"]
            .as_array()
            .filter(|s| s.len() == 1)
            .and_then(|s| s[0].as_str())
        {
            if let Some(import) = imports.values().find(|v| v["stream"] == id) {
                if let Some(path) = import["archive"].as_str() {
                    return Some(path.into());
                }
            }
        }
        self.config["history_path"].as_str().map(PathBuf::from)
    }
    pub async fn recording_start(self: &Arc<Self>, args: Value) -> Result<Value> {
        let mode = args["mode"].as_str().unwrap_or("now");
        if !matches!(mode, "now" | "earliest") {
            bail!("mode must be now or earliest");
        }
        let stream = args["stream"].as_str().context("select one stream")?;
        let _permit = self
            .recording_gate
            .try_lock()
            .context("busy: another recording command is starting")?;
        {
            let recordings = self.recordings.lock().unwrap();
            if recordings.len() >= 64 {
                bail!("recording limit reached (64 per runtime)");
            }
            if recordings
                .values()
                .filter(|r| {
                    !matches!(
                        r.status.lock().unwrap()["state"].as_str(),
                        Some("stopped" | "failed")
                    )
                })
                .count()
                >= 4
            {
                bail!("busy: at most four active recordings");
            }
        }
        // Pin the Core head before doing any filesystem work. No browser time
        // or rounded JavaScript sequence number participates in this boundary.
        let boundary = self
            .client
            .request("stream.resolve", json!({"alias":stream}))
            .await?;
        let stream = boundary["id"].as_str().context("stream id")?.to_owned();
        let epoch = boundary["epoch"].as_str().context("epoch")?.to_owned();
        let head = boundary["head"].as_u64().context("head")?;
        let snapshot = if mode == "earliest" {
            Some(self.snapshot(&json!({"streams":[stream]})).await?)
        } else {
            None
        };
        let id = uuid::Uuid::new_v4().to_string();
        let directory = self.working_dir("recordings")?.join(&id);
        std::fs::create_dir(&directory)?;
        let path = directory.join("logs.log");
        let sqlite = directory.join("logs.sqlite");
        let recording = Arc::new(Recording {
            status: Mutex::new(
                json!({"id":id,"created_at":log_proto::now_ns().to_string(),"stream":stream,"mode":mode,"state":"starting","path":path,"archive":sqlite,"boundary":head.to_string(),"bytes":"0","records":"0","coverage":snapshot.as_ref().map(|s|s.coverage.clone())}),
            ),
            stop: AtomicBool::new(false),
            done: tokio::sync::Notify::new(),
            path: path.clone(),
        });
        self.recordings
            .lock()
            .unwrap()
            .insert(id.clone(), recording.clone());
        let engine = self.clone();
        let task = recording.clone();
        tokio::spawn(async move {
            let result = engine
                .capture(&task, &stream, &epoch, head, snapshot, path, sqlite)
                .await;
            let mut status = task.status.lock().unwrap();
            match result {
                Ok(()) => status["state"] = json!("stopped"),
                Err(e) => {
                    status["state"] = json!("failed");
                    status["error"] = json!(format!("{e:#}"));
                }
            }
            drop(status);
            task.done.notify_waiters();
            let _ = engine.updates.send(());
        });
        let status = recording.status.lock().unwrap().clone();
        Ok(status)
    }
    #[allow(clippy::too_many_arguments)]
    async fn capture(
        &self,
        job: &Recording,
        stream: &str,
        epoch: &str,
        head: u64,
        snapshot: Option<Snapshot>,
        path: PathBuf,
        sqlite: PathBuf,
    ) -> Result<()> {
        let stream_owned = stream.to_owned();
        let epoch_owned = epoch.to_owned();
        let (mut writer, mut next, mut count, mut bytes) = tokio::task::spawn_blocking(move || -> Result<_> {
            let scope = snapshot.as_ref().and_then(|s| s.streams.iter().find(|s|s.catalog["id"] == stream_owned));
            let archive = snapshot.as_ref().and_then(|s|s.archive.as_ref());
            let first_live = scope.and_then(|s| s.live.first()).map(|r|r.seq);
            let first_archive = archive.and_then(|a| a.initial.get(&stream_owned)).filter(|c|c.epoch==epoch_owned).map(|c|c.next);
            let start = if snapshot.is_some() { first_live.into_iter().chain(first_archive).min().unwrap_or(head + 1) } else { head + 1 };
            let config = Config::parse(json!({"streams":[stream_owned],"mode":"create","file":{"format":"raw","paths":{&stream_owned:path}},"sqlite":{"path":sqlite},"fail_on_gap":false}))?;
            let mut writer = Archive::open(&config, BTreeMap::from([(stream_owned.clone(),Cursor {epoch:epoch_owned.clone(),next:start})]))?;
            let mut next = start;
            let mut count = 0u64;
            let mut bytes = 0u64;
            if let (Some(snapshot), Some(scope)) = (snapshot.as_ref(), scope) {
                let reader = snapshot.path.as_ref().filter(|_|archive.is_some()).map(|p|HistoryReader::open(p)).transpose()?;
                let mut live = scope.live.iter().peekable();
                let mut batch = VecDeque::new();
                let mut from = start;
                let mut exhausted = reader.is_none();
                loop {
                    if batch.is_empty() && !exhausted {
                        let rows = reader.as_ref().unwrap().read(archive.unwrap(), &stream_owned, &epoch_owned, from, scope.head, 64)?;
                        if rows.is_empty() { exhausted = true; } else { from = rows.last().unwrap().seq + 1; batch = rows.into(); }
                    }
                    let record = match (batch.front(),live.peek()) {
                        (Some(a), Some(b)) if b.seq < a.seq => Some((*live.next().unwrap()).clone()),
                        (Some(a), Some(b)) if b.seq == a.seq => { live.next(); batch.pop_front() },
                        (Some(_), _) => batch.pop_front(),
                        (None, Some(_)) => Some((*live.next().unwrap()).clone()),
                        _ => None,
                    };
                    let Some(record) = record else { break };
                    append(&mut writer, &record, &mut next, &mut count, &mut bytes)?;
                    if count.is_multiple_of(64) { writer.commit()?; }
                }
            }
            writer.commit()?;
            Ok((writer,next,count,bytes))
        }).await??;
        let mut stop_head = None;
        loop {
            let catalog = self
                .client
                .request("stream.resolve", json!({"alias":stream}))
                .await?;
            if catalog["epoch"] != epoch {
                bail!("stream epoch changed during recording");
            }
            if job.stop.load(Ordering::Relaxed) && stop_head.is_none() {
                stop_head = catalog["head"].as_u64();
            }
            let end = stop_head.unwrap_or(catalog["head"].as_u64().context("head")?);
            if next <= end {
                let page = self.client.read_range(stream, epoch, next, end, 64).await?;
                let records: Vec<Record> = serde_json::from_value(page["records"].clone())?;
                if records.is_empty() {
                    bail!("recording cannot read Core range {next}..{end}; data may have been evicted");
                }
                (writer, next, count, bytes) = tokio::task::spawn_blocking(move || -> Result<_> {
                    for record in records {
                        append(&mut writer, &record, &mut next, &mut count, &mut bytes)?;
                    }
                    writer.commit()?;
                    Ok((writer, next, count, bytes))
                })
                .await??;
            }
            {
                let mut status = job.status.lock().unwrap();
                status["state"] = json!(if stop_head.is_some() {
                    "stopping"
                } else {
                    "recording"
                });
                status["bytes"] = json!(bytes.to_string());
                status["records"] = json!(count.to_string());
                status["next"] = json!(next.to_string());
                status["writer"] = writer.status();
            }
            if stop_head.is_some_and(|end| next > end) {
                break;
            }
            if next > end {
                tokio::select! { _=tokio::time::sleep(Duration::from_millis(40))=>{}, _=self.data_ready.notified()=>{} }
            }
        }
        tokio::task::spawn_blocking(move || writer.commit()).await??;
        Ok(())
    }
    pub async fn recording_stop(&self, id: &str) -> Result<Value> {
        let job = self
            .recordings
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context("recording not found")?;
        job.stop.store(true, Ordering::Relaxed);
        loop {
            let notified = job.done.notified();
            let status = job.status.lock().unwrap().clone();
            if matches!(status["state"].as_str(), Some("stopped" | "failed")) {
                return Ok(status);
            }
            // Register before observing completion so a fast writer cannot lose
            // a stop acknowledgement between the check and the await.
            tokio::select! { _=notified=>{}, _=tokio::time::sleep(Duration::from_millis(50))=>{} }
        }
    }
    pub async fn stop_recordings(&self) {
        let ids: Vec<_> = self.recordings.lock().unwrap().keys().cloned().collect();
        for id in ids {
            let _ = self.recording_stop(&id).await;
        }
    }
    pub(crate) async fn manager(&self, op: &str, args: Value) -> Result<Value> {
        use log_proto::{read_json, write_json, Hello, Request, ServerMessage, PROTOCOL};
        let address = std::env::var("LOG_PRINT_MANAGER")
            .context("file import requires the log-print supervisor")?;
        let token = std::env::var("LOG_PRINT_MANAGER_TOKEN")?;
        tokio::time::timeout(Duration::from_secs(30), async {
            let socket = tokio::net::TcpStream::connect(address).await?;
            let (read, mut write) = socket.into_split();
            let mut read = AsyncBufReader::new(read);
            write_json(
                &mut write,
                &Hello {
                    protocol: PROTOCOL.into(),
                    plugin: "__manager__".into(),
                    token,
                    events: false,
                },
            )
            .await?;
            match read_json::<_, ServerMessage>(&mut read)
                .await?
                .context("manager closed")?
            {
                ServerMessage::Response { error: None, .. } => {}
                _ => bail!("manager authentication failed"),
            }
            write_json(
                &mut write,
                &Request {
                    id: 1,
                    op: op.into(),
                    args,
                },
            )
            .await?;
            match read_json::<_, ServerMessage>(&mut read)
                .await?
                .context("manager closed")?
            {
                ServerMessage::Response {
                    error: None,
                    result,
                    ..
                } => Ok(result),
                ServerMessage::Response { error: Some(e), .. } => Err(e.into()),
                _ => bail!("invalid manager response"),
            }
        })
        .await?
    }
}
fn append(
    writer: &mut Archive,
    record: &Record,
    next: &mut u64,
    count: &mut u64,
    bytes: &mut u64,
) -> Result<()> {
    if record.seq < *next {
        return Ok(());
    }
    if record.seq > *next {
        writer.gap(&Gap {
            stream: record.stream.clone(),
            epoch: record.epoch.clone(),
            from: *next,
            to: record.seq - 1,
            reason: "records unavailable during recording".into(),
        })?;
    }
    writer.accept(record)?;
    *next = record.seq.checked_add(1).context("sequence exhausted")?;
    *count += 1;
    *bytes += record.payload.len() as u64;
    Ok(())
}
