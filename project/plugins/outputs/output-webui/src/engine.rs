use crate::{
    lines::{self, Lines},
    store::Store,
};
use anyhow::{bail, Context, Result};
use log_plugin_sdk::Client;
use log_proto::Record;
use output_file::{HistoryReader, HistorySnapshot};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Seek, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{broadcast, Semaphore};

pub struct Engine {
    pub client: Client,
    pub store: Mutex<Store>,
    pub catalog: Mutex<Vec<Value>>,
    pub cache: Mutex<Cache>,
    pub archive_writer: Mutex<Value>,
    pub url: String,
    pub config: Value,
    pub updates: broadcast::Sender<()>,
    archive_id: Mutex<Option<String>>,
    jobs: Mutex<BTreeMap<String, Arc<Job>>>,
    scans: Arc<Semaphore>,
    frozen: Mutex<BTreeMap<String, Frozen>>,
}
struct Frozen {
    signature: String,
    value: Value,
    bytes: usize,
}
#[derive(Default)]
pub struct Cache {
    pub streams: BTreeMap<String, VecDeque<Record>>,
    bytes: usize,
    pub errors: BTreeMap<String, String>,
}
impl Cache {
    pub fn push(&mut self, r: Record) {
        self.bytes += cache_size(&r);
        let q = self.streams.entry(r.stream.clone()).or_default();
        q.push_back(r);
        let mut bytes: usize = q.iter().map(cache_size).sum();
        while q.len() > 4096 || bytes > 4 * 1024 * 1024 {
            let r = q.pop_front().unwrap();
            bytes -= cache_size(&r);
            self.bytes -= cache_size(&r);
        }
        while self.bytes > 64 * 1024 * 1024 {
            let key = self
                .streams
                .iter()
                .filter_map(|(s, q)| q.front().map(|r| (s.clone(), r.observed_ts_ns)))
                .min_by_key(|(_, t)| *t)
                .map(|(s, _)| s)
                .unwrap();
            let removed = self.streams.get_mut(&key).unwrap().pop_front().unwrap();
            self.bytes -= cache_size(&removed);
        }
    }
}
struct Job {
    status: Mutex<Value>,
    cancel: AtomicBool,
    path: PathBuf,
    index: Mutex<Vec<u64>>,
    count: Mutex<usize>,
}
#[derive(Clone)]
struct Scope {
    catalog: Value,
    live: Vec<Record>,
    head: u64,
}
#[derive(Clone)]
struct Snapshot {
    streams: Vec<Scope>,
    archive: Option<HistorySnapshot>,
    path: Option<PathBuf>,
    coverage: Value,
}
impl Engine {
    pub fn new(client: Client, store: Store, url: String, config: Value) -> Self {
        let (updates, _) = broadcast::channel(32);
        Self {
            client,
            store: Mutex::new(store),
            catalog: Mutex::new(vec![]),
            cache: Mutex::new(Cache::default()),
            archive_writer: Mutex::new(Value::Null),
            url,
            config,
            updates,
            archive_id: Mutex::new(None),
            jobs: Mutex::new(BTreeMap::new()),
            scans: Arc::new(Semaphore::new(2)),
            frozen: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn cancel_all(&self) {
        for job in self.jobs.lock().unwrap().values() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    pub fn state(&self) -> Value {
        let mut state = self.store.lock().unwrap().state.clone();
        state["url"] = json!(self.url);
        state["streams"] = json!(*self.catalog.lock().unwrap());
        state["runtime_id"] = self.config["runtime_id"].clone();
        state["archive_enabled"] = json!(self.config["history_path"].is_string());
        state["archive_writer"] = self.archive_writer.lock().unwrap().clone();
        state["errors"] = json!(self.cache.lock().unwrap().errors);
        state
    }
    fn normalize(&self, args: &mut Value) -> Result<()> {
        Self::time_bounds(args)?;
        if let Some(streams) = args.get_mut("streams") {
            let list = streams.as_array_mut().context("streams must be an array")?;
            let catalog = self.catalog.lock().unwrap();
            for binding in list {
                if let Some(key) = binding.as_str() {
                    *binding = if let Some(s) =
                        catalog.iter().find(|s| s["id"] == key || s["alias"] == key)
                    {
                        if s["alias"].is_string() {
                            json!({"owner":s["owner"],"alias":s["alias"]})
                        } else {
                            json!({"owner":s["owner"],"alias":null,"stream":s["id"],"epoch":s["epoch"]})
                        }
                    } else {
                        json!({"owner":"","alias":key})
                    };
                }
            }
        }
        Ok(())
    }
    fn time_bounds(args: &mut Value) -> Result<()> {
        for key in ["time_from", "time_end"] {
            if let Some(value) = args.get_mut(key) {
                if !value.is_null() && value.as_str() != Some("") {
                    let time = lines::time_bound(value)
                        .context("time bound must be unsigned Unix nanoseconds")?;
                    *value = json!(time.to_string());
                }
            }
        }
        Ok(())
    }
    pub async fn command(
        self: &Arc<Self>,
        method: &str,
        mut args: Value,
        budget: usize,
    ) -> Result<Value> {
        match method {
            "url" => Ok(json!({"url":self.url})),
            "streams" => list_page(
                json!({"streams":*self.catalog.lock().unwrap()}),
                "streams",
                &args,
                budget,
            ),
            "capabilities" => Ok(
                json!({"panels":["log","curve"],"layouts":["canvas","grid"],"canvas":{"coordinates":"pixels","zoom":[0.2,2.0],"panel_width":[320,4000],"panel_height":[220,4000],"shared_viewport":true},"page_size":200,"curve_points":2000,"scans":2,"history":if self.config["history_path"].is_string(){"sqlite+memory"}else{"memory_only"},"methods":["page.list","page.get","page.create","page.set","page.clone","page.delete","page.select","panel.add","panel.get","panel.set","panel.clone","panel.remove","series.add","series.set","series.remove","layout.set","history.read","history.search","history.context","history.curve","query.get","query.cancel"]}),
            ),
            "state.get" | "status.get" => Ok(self.state()),
            "query.get" => self.query(&args, budget),
            "query.cancel" => {
                let job = self
                    .jobs
                    .lock()
                    .unwrap()
                    .get(args["query"].as_str().context("query required")?)
                    .cloned()
                    .context("query not found")?;
                job.cancel.store(true, Ordering::Relaxed);
                Ok(json!({"cancel_requested":true}))
            }
            "history.read" | "history.search" | "history.context" | "history.curve" => {
                self.start_query(method, args).await
            }
            "panel.data" => {
                let engine = self.clone();
                tokio::task::spawn_blocking(move || engine.panel_data(&args, budget)).await?
            }
            _ => {
                self.normalize(&mut args)?;
                let result = self.store.lock().unwrap().command(method, &args)?;
                if method == "page.list" {
                    return list_page(result, "pages", &args, budget);
                }
                if !method.ends_with(".get") && method != "page.list" {
                    self.prune_frozen();
                    let _ = self.updates.send(());
                }
                if serde_json::to_vec(&result)?.len() > budget
                    && !method.ends_with(".get")
                    && method != "page.list"
                {
                    return Ok(
                        json!({"revision":result["revision"],"result":result["result"].get("id").map(|id|json!({"id":id})),"committed":true,"state_omitted":true}),
                    );
                }
                if serde_json::to_vec(&result)?.len() > budget {
                    bail!("reply exceeds frame budget; use HTTP");
                }
                Ok(result)
            }
        }
    }
    fn bindings(catalog: &[Value], settings: &Value) -> Vec<String> {
        let Some(bindings) = settings["streams"].as_array().filter(|v| !v.is_empty()) else {
            return catalog
                .iter()
                .filter_map(|s| s["id"].as_str().map(str::to_owned))
                .collect();
        };
        catalog
            .iter()
            .filter(|s| {
                bindings.iter().any(|b| {
                    if let Some(key) = b.as_str() {
                        s["id"] == key || s["alias"] == key
                    } else {
                        (b["owner"] == "" || b["owner"] == s["owner"])
                            && b["alias"] == s["alias"]
                            && b.get("stream").is_none_or(|id| id == &s["id"])
                            && b.get("epoch").is_none_or(|epoch| epoch == &s["epoch"])
                    }
                })
            })
            .filter_map(|s| s["id"].as_str().map(str::to_owned))
            .collect()
    }
    fn panel(&self, args: &Value) -> Result<Value> {
        let store = self.store.lock().unwrap();
        let key = args["page"]
            .as_str()
            .or_else(|| store.state["selected"].as_str())
            .context("page")?;
        let page = store.state["pages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == key || p["name"] == key)
            .context("Page not found")?;
        Ok(page["panels"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == args["panel"] || p["title"] == args["panel"])
            .context("panel not found")?
            .clone())
    }
    fn panel_data(&self, args: &Value, budget: usize) -> Result<Value> {
        let panel = self.panel(args)?;
        let id = panel["id"].as_str().context("panel id")?;
        if panel["paused"] != true {
            self.frozen.lock().unwrap().remove(id);
            return self.render_panel(&panel, budget);
        }
        let signature = serde_json::to_string(
            &json!({"streams":panel["streams"],"channels":panel["channels"],"text":panel["text"],"regex":panel["regex"],"time_from":panel["time_from"],"time_end":panel["time_end"],"mode":panel["mode"],"query":panel["query"],"offset":panel["offset"],"series":panel["series"].as_array().map(|series|series.iter().map(|s|json!({"id":s["id"],"streams":s["streams"],"channels":s["channels"],"regex":s["regex"],"field":s["field"]})).collect::<Vec<_>>())}),
        )?;
        if let Some(view) = self
            .frozen
            .lock()
            .unwrap()
            .get(id)
            .filter(|v| v.signature == signature)
        {
            return self.frozen_reply(view.value.clone(), &panel, budget);
        }
        let value = if panel["mode"] == "history" && panel["query"].is_string() {
            let mut value = self.query(
                &json!({"query":panel["query"],"offset":panel["offset"]}),
                4 * 1024 * 1024,
            )?;
            if panel["kind"] == "curve" && value["status"]["state"] == "complete" {
                while value["next"].as_u64().unwrap_or(0) < value["total"].as_u64().unwrap_or(0)
                    && value["rows"].as_array().unwrap().len() < 2000
                {
                    let next = self.query(
                        &json!({"query":panel["query"],"offset":value["next"]}),
                        4 * 1024 * 1024,
                    )?;
                    value["rows"]
                        .as_array_mut()
                        .unwrap()
                        .extend(next["rows"].as_array().unwrap().iter().cloned());
                    value["next"] = next["next"].clone();
                }
            }
            value
        } else {
            self.render_panel(&panel, 4 * 1024 * 1024)?
        };
        let bytes = serde_json::to_vec(&value)?.len();
        let mut views = self.frozen.lock().unwrap();
        if views.get(id).is_none_or(|v| v.signature != signature) {
            let used: usize = views
                .iter()
                .filter(|(key, _)| key.as_str() != id)
                .map(|(_, v)| v.bytes)
                .sum();
            if used + bytes > 64 * 1024 * 1024 {
                bail!("paused display budget reached (64 MiB); resume another panel");
            }
            views.insert(
                id.to_owned(),
                Frozen {
                    signature,
                    value,
                    bytes,
                },
            );
        }
        let value = views[id].value.clone();
        drop(views);
        self.frozen_reply(value, &panel, budget)
    }
    fn frozen_reply(&self, mut value: Value, panel: &Value, budget: usize) -> Result<Value> {
        if let Some(series) = value["series"].as_array_mut() {
            for displayed in series {
                if let Some(definition) = panel["series"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|s| s["id"] == displayed["id"])
                {
                    displayed["name"] = definition["name"].clone();
                }
            }
        }
        if let Some(query) = panel["query"].as_str() {
            if let Some(job) = self.jobs.lock().unwrap().get(query) {
                value["status"] = job.status.lock().unwrap().clone();
            }
        }
        value["frozen"] = json!(true);
        bounded(value, budget)
    }
    fn prune_frozen(&self) {
        let state = self.store.lock().unwrap().state.clone();
        let ids: Vec<_> = state["pages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p["panels"].as_array().unwrap())
            .filter(|p| p["paused"] == true)
            .filter_map(|p| p["id"].as_str())
            .collect();
        self.frozen
            .lock()
            .unwrap()
            .retain(|id, _| ids.contains(&id.as_str()));
    }
    fn render_panel(&self, panel: &Value, budget: usize) -> Result<Value> {
        let catalog = self.catalog.lock().unwrap().clone();
        let ids = Self::bindings(&catalog, panel);
        let cache = self.cache.lock().unwrap();
        let mut rows = Vec::new();
        for id in &ids {
            if let Some(records) = cache.streams.get(id) {
                let mut decoder = Lines::default();
                for r in records {
                    rows.extend(decoder.push(r));
                }
                rows.extend(decoder.flush());
            }
        }
        drop(cache);
        rows.sort_by_key(|r| {
            r["time"]
                .as_str()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
        });
        let re = compile(panel)?;
        rows.retain(|r| {
            (panel["kind"] == "curve" && r["gap"] == true && lines::curve_matches(r, panel))
                || lines::matches(r, panel, re.as_ref())
        });
        if panel["kind"] == "curve" {
            let mut series = Vec::new();
            let limit = 2000 / panel["series"].as_array().context("series")?.len().max(1);
            for s in panel["series"].as_array().context("series")? {
                let re = compile(s)?;
                let ids = Self::bindings(&catalog, s);
                let points: Vec<_> = rows
                    .iter()
                    .filter(|r| ids.iter().any(|id| r["stream"] == *id))
                    .filter(|r| lines::curve_matches(r, s))
                    .filter_map(|r| point(r, s, re.as_ref()))
                    .collect();
                series.push(json!({"id":s["id"],"name":s["name"],"points":reduce(points,limit)}));
            }
            return bounded(
                json!({"series":series,"waiting":ids.is_empty(),"history":"live_cache"}),
                budget,
            );
        }
        let start = rows.len().saturating_sub(200);
        let rows = rows.split_off(start);
        bounded(
            json!({"rows":rows,"waiting":ids.is_empty(),"history":"live_cache","retained_rows":200}),
            budget,
        )
    }
    async fn snapshot(&self, args: &Value) -> Result<Snapshot> {
        let catalog = self.client.streams().await?;
        let catalog = catalog.as_array().context("catalog")?.clone();
        let ids = Self::bindings(&catalog, args);
        let path = self.config["history_path"].as_str().map(PathBuf::from);
        let (archive, error) = if let Some(p) = path.clone() {
            tokio::task::spawn_blocking(move || {
                match HistoryReader::open(&p).and_then(|r| r.snapshot()) {
                    Ok(s) => (Some(s), None),
                    Err(e) => (None, Some(format!("{e:#}"))),
                }
            })
            .await?
        } else {
            (None, None)
        };
        if let Some(snapshot) = &archive {
            let mut identity = self.archive_id.lock().unwrap();
            if identity
                .as_ref()
                .is_some_and(|id| id != &snapshot.archive_id)
            {
                bail!("archive_identity_mismatch: archive was replaced during this runtime");
            }
            *identity = Some(snapshot.archive_id.clone());
        }
        let mut streams = Vec::new();
        let mut coverage = Vec::new();
        let mut total_bytes = 0usize;
        let mut fetched_bytes = 0usize;
        for entry in catalog
            .iter()
            .filter(|s| ids.iter().any(|id| s["id"] == *id))
        {
            let id = entry["id"].as_str().context("id")?;
            let epoch = entry["epoch"].as_str().context("epoch")?;
            let head = entry["head"].as_u64().context("head")?;
            let oldest = entry["oldest"].as_u64().unwrap_or(head + 1);
            if args["epoch"].as_str().is_some_and(|v| v != epoch) {
                bail!("epoch_mismatch: query belongs to another runtime");
            }
            let mut live: BTreeMap<u64, Record> = self
                .cache
                .lock()
                .unwrap()
                .streams
                .get(id)
                .map(|v| {
                    v.iter()
                        .filter(|r| {
                            r.epoch == epoch && (path.is_some() || r.seq >= oldest) && r.seq <= head
                        })
                        .map(|r| (r.seq, r.clone()))
                        .collect()
                })
                .unwrap_or_default();
            let mut from = oldest;
            let mut bytes = 0usize;
            let mut fetched = 0usize;
            let mut retained_bytes: usize = live.values().map(cache_size).sum();
            while from <= head
                && fetched < 4096
                && bytes < 4 * 1024 * 1024
                && fetched_bytes < 64 * 1024 * 1024
            {
                // A full WebUI cache must not prevent reading the fixed Core tail.
                // Skip cached sequences and replace its oldest records if necessary.
                if live.contains_key(&from) {
                    from = from.saturating_add(1);
                    continue;
                }
                let batch = self.client.read_range(id, epoch, from, head, 64).await?;
                for r in batch["records"].as_array().context("records")? {
                    let r: Record = serde_json::from_value(r.clone())?;
                    fetched += 1;
                    bytes += cache_size(&r);
                    fetched_bytes += cache_size(&r);
                    retained_bytes += cache_size(&r);
                    if let Some(previous) = live.insert(r.seq, r) {
                        retained_bytes -= cache_size(&previous);
                    }
                    while live.len() > 4096 || retained_bytes > 4 * 1024 * 1024 {
                        retained_bytes -= cache_size(&live.pop_first().unwrap().1);
                    }
                }
                let next = batch["next"].as_u64().context("next")?;
                if next <= from {
                    break;
                }
                from = next;
            }
            while retained_bytes > (64 * 1024 * 1024usize).saturating_sub(total_bytes) {
                retained_bytes -= cache_size(&live.pop_first().unwrap().1);
            }
            total_bytes += retained_bytes;
            let compatible = archive
                .as_ref()
                .and_then(|a| a.committed.get(id))
                .filter(|c| c.epoch == epoch);
            let archived=compatible.map(|c|json!({"first":archive.as_ref().unwrap().initial[id].next.to_string(),"last":c.next.saturating_sub(1).to_string(),"empty":c.next==archive.as_ref().unwrap().initial[id].next}));
            let memory_first = live.keys().next().copied();
            let memory_last = live.keys().next_back().copied();
            let mut ranges: Vec<(u64, u64)> = Vec::new();
            for seq in live.keys().copied() {
                if let Some((_, last)) = ranges
                    .last_mut()
                    .filter(|(_, last)| last.saturating_add(1) == seq)
                {
                    *last = seq;
                } else {
                    ranges.push((seq, seq));
                }
            }
            let range_count = ranges.len();
            let ranges: Vec<_> = ranges
                .into_iter()
                .take(32)
                .map(|(first, last)| json!({"first":first.to_string(),"last":last.to_string()}))
                .collect();
            let archive_end = compatible.map(|c| c.next.saturating_sub(1)).unwrap_or(0);
            let first = compatible
                .map(|_| archive.as_ref().unwrap().initial[id].next)
                .into_iter()
                .chain(memory_first)
                .min()
                .unwrap_or(head + 1);
            let prefix = (first > 1).then(|| json!({"first":"1","last":(first-1).to_string()}));
            let pending = if archive_end < head {
                Some(json!({"first":(archive_end+1).to_string(),"last":head.to_string()}))
            } else {
                None
            };
            let gap = memory_first
                .filter(|f| *f > archive_end + 1 && archive_end < head)
                .map(|f| json!({"first":(archive_end+1).to_string(),"last":(f-1).to_string()}));
            coverage.push(json!({"stream":id,"epoch":epoch,"head":head.to_string(),"archived":archived,"memory":{"first":memory_first.map(|n|n.to_string()),"last":memory_last.map(|n|n.to_string()),"ranges":ranges,"range_count":range_count,"ranges_truncated":range_count>32},"uncovered_prefix":prefix,"uncovered_between":gap,"uncovered_after":memory_last.filter(|n|*n<head && archive_end<head).map(|n|json!({"first":(n+1).to_string(),"last":head.to_string()})),"uncommitted":pending,"history_available":compatible.is_some(),"wrong_archive_epoch":archive.as_ref().is_some_and(|a|a.committed.contains_key(id) && compatible.is_none())}));
            streams.push(Scope {
                catalog: entry.clone(),
                live: live.into_values().collect(),
                head,
            });
        }
        let mut writer = Value::Null;
        if let Some(plugin) = self.config["history_plugin"].as_str() {
            writer = self
                .client
                .request("plugin.status", json!({"plugin":plugin}))
                .await
                .unwrap_or_else(|e| json!({"state":"unavailable","error":format!("{e:#}")}));
        }
        let coverage = json!({"mode":if path.is_some(){"archive_and_memory"}else{"memory_only"},"archive_id":archive.as_ref().map(|a|&a.archive_id),"archive_error":error,"writer":writer,"streams":coverage,"gaps":archive.as_ref().map(|a|a.gaps.iter().map(|g|json!({"stream":g.stream,"epoch":g.epoch,"from":g.from.to_string(),"to":g.to.to_string(),"reason":g.reason})).collect::<Vec<_>>()),"gap_count":archive.as_ref().map(|a|a.gap_count),"gaps_truncated":archive.as_ref().is_some_and(|a|a.gap_count>a.gaps.len()),"runtime_match":archive.as_ref().map(|a|catalog.iter().any(|s|a.committed.get(s["id"].as_str().unwrap_or("")).is_some_and(|c|s["epoch"]==c.epoch))),"fixed_boundary":true});
        Ok(Snapshot {
            streams,
            archive,
            path,
            coverage,
        })
    }
    async fn start_query(self: &Arc<Self>, method: &str, args: Value) -> Result<Value> {
        let mut args = args;
        Self::time_bounds(&mut args)?;
        if args["panel"].is_string() {
            let panel = self.panel(&args)?;
            let mut settings = panel.clone();
            for (key, value) in args.as_object().context("query args")? {
                settings[key] = value.clone();
            }
            if method == "history.curve" && args["series"].is_string() {
                settings["series"] = json!(panel["series"]
                    .as_array()
                    .context("series")?
                    .iter()
                    .filter(|s| s["id"] == args["series"] || s["name"] == args["series"])
                    .cloned()
                    .collect::<Vec<_>>());
            }
            args = settings;
        }
        let permit = self
            .scans
            .clone()
            .try_acquire_owned()
            .context("busy: two scans are already running")?;
        let snapshot = self.snapshot(&args).await?;
        let directory = PathBuf::from(self.config["state_path"].as_str().context("state_path")?)
            .parent()
            .context("parent")?
            .join("queries");
        std::fs::create_dir_all(&directory)?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = directory.join(format!("{id}.jsonl"));
        File::create(&path)?;
        let job = Arc::new(Job {
            status: Mutex::new(
                json!({"id":id,"state":"running","scanned":0,"waiting":snapshot.streams.is_empty(),"coverage":snapshot.coverage}),
            ),
            cancel: AtomicBool::new(false),
            path,
            index: Mutex::new(Vec::new()),
            count: Mutex::new(0),
        });
        {
            let mut jobs = self.jobs.lock().unwrap();
            if jobs.len() >= 32 {
                let expired = jobs
                    .iter()
                    .find(|(_, j)| j.status.lock().unwrap()["state"] != "running")
                    .map(|(id, _)| id.clone())
                    .context("too many active queries")?;
                if let Some(j) = jobs.remove(&expired) {
                    let _ = std::fs::remove_file(&j.path);
                }
            }
            jobs.insert(id.clone(), job.clone());
        }
        let method = method.to_owned();
        let engine = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let result = scan(&job, &snapshot, &method, &args);
            let mut status = job.status.lock().unwrap();
            match result {
                Ok(()) => {
                    status["state"] = json!(if job.cancel.load(Ordering::Relaxed) {
                        "cancelled"
                    } else {
                        "complete"
                    })
                }
                Err(e) => {
                    status["state"] = json!("failed");
                    status["error"] = json!(format!("{e:#}"));
                }
            }
            drop(status);
            let _ = engine.updates.send(());
        });
        Ok(json!({"query":id,"state":"running"}))
    }
    fn query(&self, args: &Value, budget: usize) -> Result<Value> {
        let job = self
            .jobs
            .lock()
            .unwrap()
            .get(args["query"].as_str().context("query required")?)
            .cloned()
            .context("query not found")?;
        let status = job.status.lock().unwrap().clone();
        let count = *job.count.lock().unwrap();
        let offset = args["offset"].as_u64().unwrap_or(0) as usize;
        let limit = args["limit"].as_u64().unwrap_or(200).clamp(1, 200) as usize;
        let block = offset / 200;
        let position = job.index.lock().unwrap().get(block).copied();
        let mut rows = Vec::new();
        if let Some(position) = position {
            let mut file = File::open(&job.path)?;
            file.seek(std::io::SeekFrom::Start(position))?;
            let mut reader = BufReader::new(file);
            let mut line = String::new();
            for i in block * 200..(offset + limit).min(count) {
                line.clear();
                if reader.read_line(&mut line)? == 0 {
                    break;
                }
                if i >= offset {
                    rows.push(serde_json::from_str::<Value>(&line)?);
                }
            }
        }
        let mut result = json!({"query":status["id"],"status":status,"total":count,"offset":offset,"next":offset,"rows":[]});
        let mut encoded_size = serde_json::to_vec(&result)?.len() + 32;
        if encoded_size > budget {
            bail!("coverage exceeds frame budget; use HTTP");
        }
        let mut selected = Vec::new();
        for row in rows {
            let size = serde_json::to_vec(&row)?.len() + usize::from(!selected.is_empty());
            if encoded_size.saturating_add(size) > budget {
                if selected.is_empty() {
                    bail!("one result exceeds reply budget; use HTTP for this query");
                }
                break;
            }
            encoded_size += size;
            selected.push(row);
        }
        result["next"] = json!(offset + selected.len());
        result["rows"] = Value::Array(selected);
        Ok(result)
    }
}
fn cache_size(r: &Record) -> usize {
    r.payload.len()
        + r.key.len()
        + r.stream.len()
        + r.epoch.len()
        + r.channel.as_ref().map_or(0, String::len)
        + 512
        + r.upstream.keys().map(|k| k.len() + 128).sum::<usize>()
        + r.upstream_epochs
            .iter()
            .map(|(k, v)| k.len() + v.len() + 128)
            .sum::<usize>()
}
fn compile(settings: &Value) -> Result<Option<regex::Regex>> {
    settings["regex"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| {
            regex::RegexBuilder::new(s)
                .size_limit(2 * 1024 * 1024)
                .build()
                .map_err(Into::into)
        })
        .transpose()
}
fn bounded(mut value: Value, budget: usize) -> Result<Value> {
    if serde_json::to_vec(&value)?.len() <= budget {
        return Ok(value);
    }
    let key = if value["rows"].is_array() {
        "rows"
    } else {
        bail!("reply exceeds frame budget; reduce series or use HTTP");
    };
    while serde_json::to_vec(&value)?.len() > budget {
        if value[key].as_array_mut().unwrap().pop().is_none() {
            bail!("reply exceeds frame budget");
        }
        value["truncated"] = json!(true);
    }
    Ok(value)
}
fn point(row: &Value, s: &Value, re: Option<&regex::Regex>) -> Option<Value> {
    let location = json!({"stream":row["stream"],"epoch":row["epoch"],"seq":row["seq"],"offset":row["offset"],"end_seq":row["end_seq"],"end_offset":row["end_offset"],"channel":row["channel"],"observed_ts_ns":row["observed_ts_ns"]});
    if row["gap"] == true {
        return Some(json!({"time":row["time"],"value":null,"gap":true,"location":location}));
    }
    lines::numeric(row, s, re).map(|v| json!({"time":row["time"],"value":v,"location":location}))
}
fn reduce(points: Vec<Value>, limit: usize) -> Vec<Value> {
    if points.len() <= limit {
        return points;
    }
    let size = points.len().div_ceil((limit / 5).max(1));
    let mut result = Vec::new();
    for chunk in points.chunks(size) {
        let mut indices = vec![0, chunk.len() - 1];
        if let Some(i) = chunk
            .iter()
            .enumerate()
            .filter(|(_, p)| p["value"].as_f64().is_some())
            .min_by(|(_, a), (_, b)| {
                a["value"]
                    .as_f64()
                    .partial_cmp(&b["value"].as_f64())
                    .unwrap()
            })
            .map(|(i, _)| i)
        {
            indices.push(i);
        }
        if let Some(i) = chunk
            .iter()
            .enumerate()
            .filter(|(_, p)| p["value"].as_f64().is_some())
            .max_by(|(_, a), (_, b)| {
                a["value"]
                    .as_f64()
                    .partial_cmp(&b["value"].as_f64())
                    .unwrap()
            })
            .map(|(i, _)| i)
        {
            indices.push(i);
        }
        if let Some(i) = chunk.iter().position(|p| p["gap"] == true) {
            indices.push(i);
        }
        indices.sort_unstable();
        indices.dedup();
        result.extend(indices.into_iter().map(|i| chunk[i].clone()));
    }
    result.truncate(limit);
    result
}
struct Sink<'a> {
    job: &'a Job,
    file: File,
    count: usize,
}
impl<'a> Sink<'a> {
    fn write(&mut self, row: &Value) -> Result<()> {
        if self.count.is_multiple_of(200) {
            self.job
                .index
                .lock()
                .unwrap()
                .push(self.file.stream_position()?);
        }
        if self.file.stream_position()? > 256 * 1024 * 1024 {
            bail!("query scratch limit reached (256 MiB); narrow the requested range");
        }
        serde_json::to_writer(&mut self.file, row)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.count += 1;
        *self.job.count.lock().unwrap() = self.count;
        Ok(())
    }
}
fn scan(job: &Job, snapshot: &Snapshot, method: &str, args: &Value) -> Result<()> {
    let reader = snapshot
        .path
        .as_ref()
        .filter(|_| snapshot.archive.is_some())
        .map(|p| HistoryReader::open(p))
        .transpose()?;
    let re = compile(args)?;
    if method == "history.curve"
        && !args["series"].is_array()
        && args["field"].as_str().unwrap_or("").is_empty()
        && !re
            .as_ref()
            .is_some_and(|r| r.capture_names().any(|n| n == Some("value")))
    {
        bail!("curve requires --field or regex with a named value group");
    }
    let mut sink = Sink {
        job,
        file: OpenOptions::new().write(true).open(&job.path)?,
        count: 0,
    };
    let mut scanned = 0u64;
    let definitions: Vec<Value> = if method == "history.curve" {
        args["series"]
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![args.clone()])
    } else {
        vec![]
    };
    let mut compiled = Vec::new();
    for definition in &definitions {
        let regex = compile(definition)?;
        if definition["field"].as_str().unwrap_or("").is_empty()
            && !regex
                .as_ref()
                .is_some_and(|r| r.capture_names().any(|n| n == Some("value")))
        {
            bail!("series requires a JSON field or named value capture");
        }
        compiled.push(regex);
    }
    let catalogs: Vec<_> = snapshot.streams.iter().map(|s| s.catalog.clone()).collect();
    let bindings: Vec<_> = definitions
        .iter()
        .map(|d| Engine::bindings(&catalogs, d))
        .collect();
    let mut curves: Vec<Vec<Value>> = definitions.iter().map(|_| Vec::new()).collect();
    for scope in &snapshot.streams {
        let stream = scope.catalog["id"].as_str().unwrap();
        let epoch = scope.catalog["epoch"].as_str().unwrap();
        let mut decoder = Lines::default();
        let mut window = ContextWindow::new(args);
        let mut from = args["from"].as_u64().unwrap_or(1);
        let end = args["end"].as_u64().unwrap_or(scope.head).min(scope.head);
        let start = from;
        let mut live = scope
            .live
            .iter()
            .filter(|r| r.seq >= start && r.seq <= end)
            .peekable();
        let archive = snapshot
            .archive
            .as_ref()
            .filter(|s| s.committed.get(stream).is_some_and(|c| c.epoch == epoch));
        let mut batch = VecDeque::new();
        let mut archive_done = archive.is_none();
        loop {
            if job.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            if batch.is_empty() && !archive_done {
                let rows = reader.as_ref().unwrap().read(
                    archive.unwrap(),
                    stream,
                    epoch,
                    from,
                    end,
                    64,
                )?;
                if rows.is_empty() {
                    archive_done = true;
                } else {
                    from = rows.last().unwrap().seq.saturating_add(1);
                    batch = rows.into();
                }
            }
            let record = match (batch.front(), live.peek()) {
                (Some(a), Some(b)) if b.seq < a.seq => Some((*live.next().unwrap()).clone()),
                (Some(a), Some(b)) if a.seq == b.seq => {
                    live.next();
                    batch.pop_front()
                }
                (Some(_), _) => batch.pop_front(),
                (None, Some(_)) => Some((*live.next().unwrap()).clone()),
                _ => None,
            };
            let Some(record) = record else {
                break;
            };
            scanned += 1;
            for row in decoder.push(&record) {
                if job.cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                process(&row, method, args, re.as_ref(), &mut sink, &mut window)?;
                curve_row(&row, args, &definitions, &compiled, &bindings, &mut curves);
            }
            if scanned.is_multiple_of(64) {
                job.status.lock().unwrap()["scanned"] = json!(scanned);
            }
            for curve in &mut curves {
                if curve.len() > 8000 {
                    *curve = reduce(std::mem::take(curve), 4000);
                }
            }
        }
        for row in decoder.flush() {
            process(&row, method, args, re.as_ref(), &mut sink, &mut window)?;
            curve_row(&row, args, &definitions, &compiled, &bindings, &mut curves);
        }
    }
    if method == "history.curve" {
        let limit = 2000 / curves.len().max(1);
        for (i, curve) in curves.into_iter().enumerate() {
            for mut p in reduce(curve, limit) {
                p["series"] = json!(definitions[i]["id"]
                    .as_str()
                    .or(definitions[i]["name"].as_str())
                    .unwrap_or("value"));
                p["name"] = json!(definitions[i]["name"].as_str().unwrap_or("value"));
                sink.write(&p)?;
            }
        }
    }
    job.status.lock().unwrap()["scanned"] = json!(scanned);
    Ok(())
}
fn process(
    row: &Value,
    method: &str,
    args: &Value,
    re: Option<&regex::Regex>,
    sink: &mut Sink<'_>,
    window: &mut ContextWindow,
) -> Result<()> {
    if method == "history.context" {
        window.push(row, sink)?;
        return Ok(());
    }
    if !lines::matches(row, args, if method == "history.curve" { None } else { re }) {
        return Ok(());
    }
    if method != "history.curve" {
        sink.write(row)?;
    }
    Ok(())
}

fn curve_row(
    row: &Value,
    args: &Value,
    definitions: &[Value],
    compiled: &[Option<regex::Regex>],
    bindings: &[Vec<String>],
    curves: &mut [Vec<Value>],
) {
    if !lines::curve_matches(row, args) {
        return;
    }
    for (i, definition) in definitions.iter().enumerate() {
        if bindings[i].iter().any(|s| row["stream"] == *s) && lines::curve_matches(row, definition)
        {
            if let Some(p) = point(row, definition, compiled[i].as_ref()) {
                curves[i].push(p);
            }
        }
    }
}

struct ContextWindow {
    seq: Option<u64>,
    offset: Option<usize>,
    before: VecDeque<Value>,
    limit: usize,
    after: usize,
    found: bool,
}
impl ContextWindow {
    fn new(args: &Value) -> Self {
        Self {
            seq: args["seq"]
                .as_u64()
                .or_else(|| args["seq"].as_str().and_then(|s| s.parse().ok())),
            offset: args["byte_offset"].as_u64().map(|n| n as usize),
            before: VecDeque::new(),
            limit: args["before"].as_u64().unwrap_or(10).min(100) as usize,
            after: args["after"].as_u64().unwrap_or(10).min(100) as usize,
            found: false,
        }
    }
    fn push(&mut self, row: &Value, sink: &mut Sink<'_>) -> Result<()> {
        let target = self.seq.context("seq required")?;
        if self.found {
            if self.after > 0 {
                sink.write(row)?;
                self.after -= 1;
            }
            return Ok(());
        }
        let seq = row["seq"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let end = row["end_seq"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(seq);
        let contains = seq <= target
            && target <= end
            && self.offset.is_none_or(|o| {
                (seq != target || o >= row["offset"].as_u64().unwrap_or(0) as usize)
                    && (end != target
                        || o < row["end_offset"].as_u64().unwrap_or(u64::MAX) as usize)
            });
        if contains {
            for before in self.before.drain(..) {
                sink.write(&before)?;
            }
            let mut hit = row.clone();
            hit["context_target"] = json!(true);
            sink.write(&hit)?;
            self.found = true;
        } else {
            self.before.push_back(row.clone());
            if self.before.len() > self.limit {
                self.before.pop_front();
            }
        }
        Ok(())
    }
}

fn list_page(mut value: Value, key: &str, args: &Value, budget: usize) -> Result<Value> {
    let all = value[key].as_array().context("list")?;
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let total = all.len();
    let rows: Vec<_> = all
        .iter()
        .skip(offset)
        .take(args["limit"].as_u64().unwrap_or(200).clamp(1, 200) as usize)
        .cloned()
        .collect();
    value[key] = json!([]);
    value["total"] = json!(total);
    value["next"] = json!(offset);
    value["offset"] = json!(offset);
    for row in rows {
        value[key].as_array_mut().unwrap().push(row);
        if serde_json::to_vec(&value)?.len() > budget {
            value[key].as_array_mut().unwrap().pop();
            if value[key].as_array().unwrap().is_empty() {
                bail!("one item exceeds reply budget; use HTTP");
            }
            break;
        }
    }
    value["next"] = json!(offset + value[key].as_array().unwrap().len());
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn curve_reduction_keeps_extrema_and_breaks() {
        let mut points: Vec<_> = (0..10000)
            .map(|i| json!({"time":i.to_string(),"value":1.}))
            .collect();
        points[401]["value"] = json!(-999.);
        points[450]["value"] = json!(999.);
        points[1000]["value"] = Value::Null;
        points[1000]["gap"] = json!(true);
        let points = reduce(points, 2000);
        assert!(points.len() <= 2000);
        assert!(points.iter().any(|p| p["value"] == -999.));
        assert!(points.iter().any(|p| p["value"] == 999.));
        assert!(points.iter().any(|p| p["gap"] == true));
    }
    #[test]
    fn metadata_counts_toward_cache_budget_and_global_eviction_is_exact() {
        let mut cache = Cache::default();
        for stream in 0..24 {
            for seq in 1..=9 {
                cache.push(Record {
                    stream: stream.to_string(),
                    epoch: "e".into(),
                    seq,
                    key: "key".into(),
                    payload: vec![0; 512 * 1024],
                    source_ts_ns: None,
                    observed_ts_ns: seq + stream * 10,
                    upstream: BTreeMap::new(),
                    upstream_epochs: BTreeMap::new(),
                    channel: None,
                    source_seq: None,
                });
            }
        }
        assert!(cache.bytes <= 64 * 1024 * 1024);
        assert_eq!(
            cache.bytes,
            cache
                .streams
                .values()
                .flat_map(|q| q.iter())
                .map(cache_size)
                .sum::<usize>()
        );
        assert!(
            cache
                .streams
                .values()
                .all(|q| q.len() <= 4096
                    && q.iter().map(cache_size).sum::<usize>() <= 4 * 1024 * 1024)
        );
    }
}
