use anyhow::{anyhow, bail, Context, Result};
use log_plugin_sdk::{Client, Control, Event};
use log_proto::Fault;
use output_file::{record_bytes, Archive, Config, Cursor, Gap, Mode};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc as blocking, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Debug)]
enum Stop {
    Requested,
    Failed(String),
}
#[derive(Clone)]
struct Completion {
    status: Value,
    error: Option<String>,
}
struct Counters {
    records: AtomicU64,
    bytes: AtomicU64,
    slots: Arc<Semaphore>,
    budget: Arc<Semaphore>,
    max_records: usize,
    max_bytes: usize,
}
impl Counters {
    fn new(c: &Config) -> Self {
        Self {
            records: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            slots: Arc::new(Semaphore::new(c.queue.max_records)),
            budget: Arc::new(Semaphore::new(c.queue.max_bytes)),
            max_records: c.queue.max_records,
            max_bytes: c.queue.max_bytes,
        }
    }
    fn decorate(&self, mut status: Value, state: &str) -> Value {
        status["state"] = json!(state);
        status["received_records"] = json!(self.records.load(Ordering::Relaxed));
        status["received_bytes"] = json!(self.bytes.load(Ordering::Relaxed));
        status["queue"] = json!({
            "records": self.max_records - self.slots.available_permits(),
            "bytes": self.max_bytes - self.budget.available_permits(),
            "max_records": self.max_records, "max_bytes": self.max_bytes,
            "includes_active_write": true,
            "sdk_event_capacity": log_proto::EVENT_QUEUE,
        });
        status
    }
}
// Core report accepts at most 16 KiB. Keep omissions explicit and expose
// the complete in-process snapshot through status.get on the control channel.
fn bounded_report(mut report: Value) -> Value {
    const LIMIT: usize = 16 * 1024;
    if serde_json::to_vec(&report).is_ok_and(|v| v.len() <= LIMIT) {
        return report;
    }
    let names: Vec<String> = report["common"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    report["report_truncated"] = json!(true);
    report["total_streams"] = json!(names.len());
    report["stream_details_control"] = json!("status.get");
    if let Some(streams) = report["file"]["streams"].as_object() {
        let gaps: u64 = streams
            .values()
            .filter_map(|s| s["gap_count"].as_u64())
            .sum();
        let incomplete = streams.values().filter(|s| s["incomplete"] == true).count();
        report["file"]["total_gap_count"] = json!(gaps);
        report["file"]["incomplete_streams"] = json!(incomplete);
    }
    for name in names.iter().rev() {
        if serde_json::to_vec(&report).is_ok_and(|v| v.len() <= LIMIT) {
            return report;
        }
        for pointer in [
            "/common",
            "/file/streams",
            "/sqlite/confirmed",
            "/sqlite/written",
        ] {
            if let Some(map) = report.pointer_mut(pointer).and_then(Value::as_object_mut) {
                map.remove(name);
            }
        }
    }
    if serde_json::to_vec(&report).is_ok_and(|v| v.len() <= LIMIT) {
        return report;
    }
    json!({"state":report["state"],"error":report["error"].as_str().map(|e| e.chars().take(2048).collect::<String>()),
        "queue":report["queue"],"received_records":report["received_records"],"received_bytes":report["received_bytes"],
        "report_truncated":true,"total_streams":names.len(),"stream_details_control":"status.get"})
}
enum Item {
    Record(log_proto::Record),
    Gap(Gap),
}
struct Pending {
    item: Item,
    bytes: usize,
}
struct Work {
    pending: Pending,
    _slot: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}
fn pending(event: Event, counters: &Counters) -> Result<Pending> {
    match event {
        Event::Record(record) => {
            let bytes = record_bytes(&record)?;
            counters.records.fetch_add(1, Ordering::Relaxed);
            counters.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
            Ok(Pending {
                item: Item::Record(record),
                bytes,
            })
        }
        Event::Gap {
            stream,
            epoch,
            from,
            to,
            reason,
        } => {
            let gap = Gap {
                stream,
                epoch,
                from,
                to,
                reason,
            };
            let bytes = serde_json::to_vec(&gap)?.len();
            Ok(Pending {
                item: Item::Gap(gap),
                bytes,
            })
        }
        Event::Disconnected { stream, reason } => {
            bail!("event connection lost for {stream}: {reason}; archive completeness unknown")
        }
    }
}
async fn enqueue(
    pending: Pending,
    sender: &blocking::Sender<Work>,
    counters: &Counters,
) -> Result<()> {
    if pending.bytes > counters.max_bytes {
        bail!(
            "record including metadata exceeds configured queue byte budget ({}/{})",
            pending.bytes,
            counters.max_bytes
        );
    }
    let slot = counters.slots.clone().acquire_owned().await?;
    let bytes = counters
        .budget
        .clone()
        .acquire_many_owned(pending.bytes as u32)
        .await?;
    sender
        .send(Work {
            pending,
            _slot: slot,
            _bytes: bytes,
        })
        .map_err(|_| anyhow!("archive worker stopped before accepting queued record"))
}
fn update_view(archive: &Archive, view: &Mutex<Value>) {
    *view.lock().unwrap() = archive.status();
}
fn commit(archive: &mut Archive, view: &Mutex<Value>) -> Result<()> {
    let result = archive.commit();
    update_view(archive, view);
    result
}
fn archive_worker(
    mut archive: Archive,
    config: Config,
    receiver: blocking::Receiver<Work>,
    view: Arc<Mutex<Value>>,
) -> Result<Value> {
    let delay = if std::env::var("LOG_PRINT_ARCHIVE_TESTING").as_deref() == Ok("1") {
        std::env::var("LOG_PRINT_ARCHIVE_TEST_DELAY_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        0
    };
    let mut count = 0usize;
    let mut bytes = 0usize;
    let mut started: Option<Instant> = None;
    loop {
        let received = if let Some(started) = started {
            receiver.recv_timeout(
                Duration::from_millis(config.commit.max_delay_ms).saturating_sub(started.elapsed()),
            )
        } else {
            receiver
                .recv()
                .map_err(|_| blocking::RecvTimeoutError::Disconnected)
        };
        let work = match received {
            Ok(work) => work,
            Err(blocking::RecvTimeoutError::Timeout) => {
                commit(&mut archive, &view)?;
                count = 0;
                bytes = 0;
                started = None;
                continue;
            }
            Err(blocking::RecvTimeoutError::Disconnected) => {
                commit(&mut archive, &view)?;
                return Ok(archive.status());
            }
        };
        match &work.pending.item {
            Item::Record(record) => {
                if count > 0 && bytes.saturating_add(work.pending.bytes) > config.commit.max_bytes {
                    commit(&mut archive, &view)?;
                    count = 0;
                    bytes = 0;
                    started = None;
                }
                if started.is_none() {
                    started = Some(Instant::now());
                }
                if delay > 0 {
                    std::thread::sleep(Duration::from_millis(delay));
                }
                let result = archive.accept(record);
                update_view(&archive, &view);
                result?;
                count += 1;
                bytes = bytes.saturating_add(work.pending.bytes);
                if count >= config.commit.max_records
                    || bytes >= config.commit.max_bytes
                    || started.is_some_and(|s| {
                        s.elapsed() >= Duration::from_millis(config.commit.max_delay_ms)
                    })
                {
                    commit(&mut archive, &view)?;
                    count = 0;
                    bytes = 0;
                    started = None;
                }
            }
            Item::Gap(gap) => {
                commit(&mut archive, &view)?;
                let result = archive.gap(gap);
                update_view(&archive, &view);
                result?;
                commit(&mut archive, &view)?;
                count = 0;
                bytes = 0;
                started = None;
                if config.fail_on_gap {
                    bail!(
                        "archive stopped on gap {}/{} {}..={}: {}",
                        gap.stream,
                        gap.epoch,
                        gap.from,
                        gap.to,
                        gap.reason
                    );
                }
            }
        }
    }
}

async fn controls_loop(
    client: Client,
    mut controls: mpsc::Receiver<Control>,
    config: Value,
    stop: watch::Sender<Option<Stop>>,
    mut completion: watch::Receiver<Option<Completion>>,
    counters: Arc<Counters>,
    view: Arc<Mutex<Value>>,
) -> Result<()> {
    let origins: BTreeMap<String, &str> = config
        .as_object()
        .unwrap()
        .keys()
        .map(|key| {
            (
                key.clone(),
                if client.config().get(key).is_some() {
                    "plugin_config"
                } else {
                    "built_in_default"
                },
            )
        })
        .collect();
    let mut shutdown_calls = Vec::new();
    let termination = io_plugin_util::termination();
    tokio::pin!(termination);
    let mut signaled = false;
    loop {
        tokio::select! {
            biased;
            changed = completion.changed() => {
                if changed.is_err() { return Ok(()); }
                let finished = completion.borrow().clone();
                if let Some(finished) = finished {
                    for call_id in shutdown_calls {
                        let fault = finished.error.as_ref().map(|e| Fault { code: "archive_failed".into(), message: e.clone() });
                        client.reply_control(call_id, json!({"stopped":finished.error.is_none(),"archive":finished.status}), fault).await?;
                    }
                    return Ok(());
                }
            }
            _ = &mut termination, if !signaled => {
                signaled = true;
                stop.send_if_modified(|s| { if s.is_none() { *s = Some(Stop::Requested); true } else { false } });
            }
            control = controls.recv() => {
                let Some(control) = control else {
                    stop.send_replace(Some(Stop::Failed("Core control connection lost; archive completeness unknown".into())));
                    return Ok(());
                };
                let (result, fault) = match control.method.as_str() {
                    "shutdown" if shutdown_calls.len() < 32 => {
                        shutdown_calls.push(control.call_id);
                        stop.send_if_modified(|s| { if s.is_none() { *s = Some(Stop::Requested); true } else { false } });
                        continue;
                    }
                    "config.get" => (json!({"effective":config,"origins":origins,"dynamic_fields":[],"others":"restart_required"}), None),
                    "status.get" => {
                        let value = view.lock().unwrap().clone();
                        let state = if stop.borrow().is_some() { "draining" } else if value.get("archive_id").is_some() { "archiving" } else { "initializing" };
                        (counters.decorate(value, state), None)
                    }
                    "config.patch" => (Value::Null, Some(Fault { code:"restart_required".into(), message:"all output-file business configuration requires restart".into() })),
                    _ => (Value::Null, Some(Fault { code:"invalid_control".into(), message:format!("unsupported or busy control {}", control.method) })),
                };
                if let Err(error) = client.reply_control(control.call_id, result, fault).await {
                    stop.send_replace(Some(Stop::Failed(format!("control reply failed: {error:#}; archive completeness unknown"))));
                    return Err(error);
                }
            }
        }
    }
}
async fn freeze(
    client: &Client,
    streams: &[String],
    events: &mut mpsc::Receiver<Event>,
) -> Result<()> {
    // Receiver::close is the acceptance boundary. Drain everything already
    // enqueued in the SDK, plus the pending event held by this process. A socket
    // frame not yet enqueued by the SDK remains unaccepted and is read on resume.
    events.close();
    for stream in streams {
        client.unsubscribe(stream).await?;
    }
    Ok(())
}
async fn initialize(client: &Client, config: &Config) -> Result<Archive> {
    let status = client.request("status", json!({})).await?;
    let plugin_id = std::env::var("LOG_PRINT_PLUGIN")?;
    let spec = status["config"]["plugins"]
        .as_array()
        .and_then(|p| p.iter().find(|p| p["id"] == plugin_id))
        .context("Core status omitted current plugin declaration")?;
    let reads: Vec<String> = serde_json::from_value(spec["reads"].clone())?;
    config.validate_reads(&reads)?;
    let c = config.clone();
    let prepared = tokio::task::spawn_blocking(move || Archive::prepare(&c)).await??;
    let mut initial = BTreeMap::new();
    if matches!(config.mode, Mode::Create) {
        for stream in &config.streams {
            let read = client
                .request(
                    "read",
                    json!({"stream":stream,"from":config.from,"limit":1}),
                )
                .await?;
            let epoch = read["epoch"]
                .as_str()
                .context("read omitted stream epoch")?
                .to_owned();
            let next = read["from"]
                .as_u64()
                .context("read omitted resolved start cursor")?;
            if next == 0 {
                bail!("Core did not resolve a nonzero initial cursor");
            }
            initial.insert(stream.clone(), Cursor { epoch, next });
        }
    }
    tokio::task::spawn_blocking(move || prepared.initialize(initial)).await?
}
async fn run(
    client: &Client,
    config: &Config,
    events: &mut mpsc::Receiver<Event>,
    stop: &mut watch::Receiver<Option<Stop>>,
    counters: Arc<Counters>,
    view: Arc<Mutex<Value>>,
) -> Result<Value> {
    let archive = initialize(client, config).await?;
    let cursors = archive.cursors();
    update_view(&archive, &view);
    for (stream, cursor) in &cursors {
        if stop.borrow().is_some() {
            break;
        }
        // Read checks the stored epoch even when no records are available.
        client
            .request(
                "read",
                json!({"stream":stream,"from":cursor.next,"epoch":cursor.epoch,"limit":1}),
            )
            .await?;
        client
            .subscribe_epoch(stream, cursor.next, Some(&cursor.epoch))
            .await?;
    }
    let (sender, receiver) = blocking::channel();
    let worker_config = config.clone();
    let worker_view = view.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        archive_worker(archive, worker_config, receiver, worker_view)
    });
    let mut failure = None;
    // A pending enqueue future owns an accepted SDK event and MUST finish on
    // shutdown. Cancellation is only permitted after a worker failure.
    loop {
        if stop.borrow().is_some() {
            break;
        }
        let event = tokio::select! {
            biased;
            changed = stop.changed() => { if changed.is_err() { failure=Some(anyhow!("control task disappeared")); } break; }
            result = &mut worker => {
                freeze(client, &config.streams, events).await?;
                return result.context("archive worker panicked")?;
            }
            event = events.recv() => event,
        };
        let Some(event) = event else {
            failure = Some(anyhow!(
                "SDK event queue closed; archive completeness unknown"
            ));
            break;
        };
        let item = match pending(event, &counters) {
            Ok(item) => item,
            Err(e) => {
                failure = Some(e);
                break;
            }
        };
        let enqueue = enqueue(item, &sender, &counters);
        tokio::pin!(enqueue);
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() { failure=Some(anyhow!("control task disappeared")); }
                freeze(client, &config.streams, events).await?;
                tokio::select! {
                    result = &mut worker => return result.context("archive worker panicked")?,
                    result = &mut enqueue => if let Err(e)=result { failure=Some(e); },
                }
                break;
            }
            result = &mut worker => {
                freeze(client, &config.streams, events).await?;
                return result.context("archive worker panicked")?;
            }
            result = &mut enqueue => if let Err(e) = result { failure=Some(e); break; },
        }
    }
    freeze(client, &config.streams, events).await?;
    // A disconnect/gap failure is never converted into a successful shutdown.
    if let Some(Stop::Failed(reason)) = stop.borrow().clone() {
        failure = Some(anyhow!(reason));
    }
    while let Some(event) = events.recv().await {
        let item = match pending(event, &counters) {
            Ok(item) => item,
            Err(e) => {
                if failure.is_none() {
                    failure = Some(e);
                }
                continue;
            }
        };
        tokio::select! {
            result = &mut worker => return result.context("archive worker panicked")?,
            result = enqueue(item, &sender, &counters) => if let Err(e)=result { failure=Some(e); break; },
        }
    }
    drop(sender);
    let final_status = worker.await.context("archive worker panicked")??;
    if let Some(Stop::Failed(reason)) = stop.borrow().clone() {
        failure = Some(anyhow!(reason));
    }
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(final_status)
}
#[tokio::main]
async fn main() -> Result<()> {
    let (client, mut events, controls) = log_plugin_sdk::connect_env().await?;
    let config = match Config::parse(client.config().clone()) {
        Ok(config) => config,
        Err(error) => {
            io_plugin_util::finish(&client, &Err(anyhow!("{error:#}"))).await;
            return Err(error);
        }
    };
    let effective = serde_json::to_value(&config)?;
    let counters = Arc::new(Counters::new(&config));
    let view = Arc::new(Mutex::new(json!({"state":"initializing"})));
    let (stop_tx, mut stop) = watch::channel(None);
    let (completion_tx, completion) = watch::channel(None);
    let controller = tokio::spawn(controls_loop(
        client.clone(),
        controls,
        effective,
        stop_tx,
        completion,
        counters.clone(),
        view.clone(),
    ));
    let report_client = client.clone();
    let report_view = view.clone();
    let report_counters = counters.clone();
    let report_stop = stop.clone();
    let reporter = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let value = report_view.lock().unwrap().clone();
            let state = if report_stop.borrow().is_some() {
                "draining"
            } else if value.get("targets").is_some()
                || value.get("file").is_some()
                || value.get("sqlite").is_some()
            {
                "archiving"
            } else {
                "initializing"
            };
            let report = bounded_report(report_counters.decorate(value, state));
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                report_client.request("report", report),
            )
            .await;
        }
    });
    let mut result = run(
        &client,
        &config,
        &mut events,
        &mut stop,
        counters.clone(),
        view.clone(),
    )
    .await;
    let _ = freeze(&client, &config.streams, &mut events).await;
    reporter.abort();
    let _ = reporter.await;
    let mut status = counters.decorate(
        view.lock().unwrap().clone(),
        if result.is_ok() { "stopped" } else { "failed" },
    );
    let mut error = result.as_ref().err().map(|e| format!("{e:#}"));
    if let Some(error) = &error {
        status["error"] = json!(error);
        status["complete"] = json!(false);
    }
    // Final status is sent before any successful shutdown acknowledgement.
    let final_report = tokio::time::timeout(
        Duration::from_secs(2),
        client.request("report", bounded_report(status.clone())),
    )
    .await;
    let report_error = match final_report {
        Ok(Ok(_)) => None,
        Ok(Err(e)) => Some(format!(
            "final report failed: {e:#}; control outcome/completeness unknown"
        )),
        Err(e) => Some(format!(
            "final report timed out: {e}; control outcome unknown"
        )),
    };
    if result.is_ok() {
        if let Some(reason) = report_error {
            status["state"] = json!("failed");
            status["error"] = json!(reason);
            status["complete"] = json!(false);
            error = Some(reason.clone());
            result = Err(anyhow!(reason));
        }
    }
    completion_tx.send_replace(Some(Completion { status, error }));
    match tokio::time::timeout(Duration::from_secs(5), controller).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => eprintln!("control finalization failed: {error:#}"),
        Ok(Err(error)) => eprintln!("control task failed: {error}"),
        Err(_) => eprintln!("shutdown reply not confirmed; caller must treat result as unknown"),
    }
    result.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_report_is_explicitly_bounded_and_small_report_is_unchanged() {
        let small = json!({"state":"archiving","common":{"one":{"epoch":"epoch","next":1}}});
        assert_eq!(bounded_report(small.clone()), small);
        let mut common = serde_json::Map::new();
        let mut streams = serde_json::Map::new();
        for n in 0..128 {
            let name = format!("stream-{n:03}-{}", "a".repeat(48));
            let cursor = json!({"epoch":"00000000-0000-0000-0000-000000000001","next":u64::MAX});
            common.insert(name.clone(), cursor.clone());
            streams.insert(name, json!({"confirmed":cursor,"written":cursor,"confirmed_bytes":u64::MAX,"gap_count":2,"incomplete":true}));
        }
        let full = json!({"state":"archiving","common":common,"file":{"streams":streams},"sqlite":{"confirmed":common,"written":common}});
        let bounded = bounded_report(full);
        assert!(serde_json::to_vec(&bounded).unwrap().len() <= 16384);
        assert_eq!(bounded["report_truncated"], true);
        assert_eq!(bounded["total_streams"], 128);
        assert_eq!(bounded["file"]["total_gap_count"], 256);
        assert_eq!(bounded["stream_details_control"], "status.get");
    }
}
