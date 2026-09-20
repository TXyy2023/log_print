//! Bounded numerical extraction, independent view sessions and deterministic export snapshots.
use anyhow::{anyhow, bail, ensure, Context, Result};
use base64::Engine;
use log_proto::Record;
use plotters::prelude::*;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, VecDeque},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

pub const MAX_SESSIONS: usize = 16;
pub const MAX_SERIES: usize = 8;
pub const MAX_TOTAL_POINTS: usize = 131_072;
pub const VIEW_POINTS: usize = 512;
const DEFAULT_PATTERN: &str =
    r"(?P<value>[-+]?(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:[eE][-+]?[0-9]+)?)";
const COLORS: [&str; 8] = [
    "#e76f51", "#2a9d8f", "#6389e9", "#cba052", "#a577cf", "#47a7b8", "#dc72a4", "#8ca457",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OutputConfig {
    pub streams: Vec<String>,
    pub sessions: Vec<SessionConfig>,
    pub from: u64,
    pub max_line_bytes: usize,
    pub headless: bool,
    pub tty: Option<String>,
    pub session: Option<String>,
    pub web_bind: String,
}
impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            streams: vec![],
            sessions: vec![],
            from: 1,
            max_line_bytes: 65_536,
            headless: false,
            tty: None,
            session: None,
            web_bind: "127.0.0.1:0".into(),
        }
    }
}
impl OutputConfig {
    pub fn from_value(value: &Value) -> Result<Self> {
        let mut c: Self = if value.is_null() {
            Self::default()
        } else {
            serde_json::from_value(value.clone())?
        };
        ensure!(
            !c.streams.is_empty() && c.streams.len() <= 32,
            "streams must contain 1..32 readable stream IDs"
        );
        c.streams.sort();
        c.streams.dedup();
        ensure!(
            c.streams.iter().all(|s| !s.is_empty() && s.len() <= 128),
            "invalid stream ID"
        );
        ensure!(
            (256..=1_048_576).contains(&c.max_line_bytes),
            "max_line_bytes must be 256..1048576"
        );
        if c.sessions.is_empty() {
            c.sessions.push(SessionConfig {
                series: c
                    .streams
                    .iter()
                    .take(MAX_SERIES)
                    .map(|stream| SeriesSpec {
                        name: stream.clone(),
                        stream: stream.clone(),
                        pattern: Some(DEFAULT_PATTERN.into()),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            });
        }
        Ok(c)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SeriesSpec {
    pub name: String,
    pub stream: String,
    pub pattern: Option<String>,
    pub capture: String,
    pub json_pointer: Option<String>,
}
impl Default for SeriesSpec {
    fn default() -> Self {
        Self {
            name: String::new(),
            stream: String::new(),
            pattern: None,
            capture: "value".into(),
            json_pointer: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    pub id: String,
    pub title: String,
    pub series: Vec<SeriesSpec>,
    pub window_secs: f64,
    pub max_points: usize,
    pub refresh_ms: u64,
    pub theme: String,
    pub y_min: Option<f64>,
    pub y_max: Option<f64>,
    pub paused: bool,
}
impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            id: "default".into(),
            title: "Log signals".into(),
            series: vec![],
            window_secs: 60.0,
            max_points: 2048,
            refresh_ms: 100,
            theme: "dark".into(),
            y_min: None,
            y_max: None,
            paused: false,
        }
    }
}
#[derive(Clone, Debug)]
struct Point {
    x: f64,
    y: Option<f64>,
    seq: u64,
}
struct SeriesState {
    spec: SeriesSpec,
    regex: Option<Regex>,
    points: VecDeque<Point>,
    matched: u64,
    unmatched: u64,
    invalid: u64,
}
struct SessionState {
    config: SessionConfig,
    revision: u64,
    series: Vec<SeriesState>,
    frozen: Option<Snapshot>,
}
#[derive(Default)]
struct Framer {
    bytes: Vec<u8>,
    dropping: bool,
    epoch: String,
    seq: u64,
    last_x: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamStats {
    pub records: u64,
    pub lines: u64,
    pub invalid_utf8: u64,
    pub oversized_lines: u64,
    pub gaps: u64,
    pub disconnections: u64,
    pub disconnected: bool,
    pub disconnect_reason: Option<String>,
    pub duplicate_records: u64,
    pub timestamp_regressions: u64,
    pub last_seq: u64,
    pub epoch: String,
    pub pending_bytes: usize,
    pub last_gap: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeriesSnapshot {
    pub name: String,
    pub stream: String,
    pub color: String,
    pub data: Vec<(f64, Option<f64>)>,
    pub retained_points: usize,
    pub first_seq: Option<u64>,
    pub last_seq: Option<u64>,
    pub matched: u64,
    pub unmatched: u64,
    pub invalid: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub revision: u64,
    pub generation: u64,
    pub title: String,
    pub theme: String,
    pub window_secs: f64,
    pub refresh_ms: u64,
    pub paused: bool,
    pub x_range: [f64; 2],
    pub y_range: [f64; 2],
    pub series: Vec<SeriesSnapshot>,
    pub config: SessionConfig,
    pub exported_at_ns: u64,
    pub source_status: BTreeMap<String, StreamStats>,
    pub live_source_status: BTreeMap<String, StreamStats>,
}
pub type SharedHub = Arc<Mutex<PlotHub>>;
pub struct PlotHub {
    config: OutputConfig,
    sessions: BTreeMap<String, SessionState>,
    framers: BTreeMap<String, Framer>,
    stats: BTreeMap<String, StreamStats>,
    generation: u64,
}

impl SessionState {
    fn new(config: SessionConfig, streams: &[String]) -> Result<Self> {
        ensure!(
            !config.id.is_empty()
                && config.id.len() <= 64
                && config
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)),
            "session id must be 1..64 ASCII letters/digits/-_."
        );
        ensure!(config.title.len() <= 256, "title is too long");
        ensure!(
            (1..=MAX_SERIES).contains(&config.series.len()),
            "session requires 1..8 series"
        );
        ensure!(
            (16..=8192).contains(&config.max_points),
            "max_points must be 16..8192 per series"
        );
        ensure!(
            config.window_secs.is_finite() && (0.1..=86400.0).contains(&config.window_secs),
            "window_secs must be 0.1..86400"
        );
        ensure!(
            (25..=10_000).contains(&config.refresh_ms),
            "refresh_ms must be 25..10000"
        );
        ensure!(
            matches!(config.theme.as_str(), "dark" | "light"),
            "theme must be dark or light"
        );
        ensure!(
            config.y_min.is_none_or(f64::is_finite) && config.y_max.is_none_or(f64::is_finite),
            "axis limits must be finite"
        );
        if let (Some(min), Some(max)) = (config.y_min, config.y_max) {
            ensure!(min < max, "y_min must be below y_max");
        }
        let mut names = std::collections::BTreeSet::new();
        let mut series = Vec::new();
        for spec in &config.series {
            ensure!(
                !spec.name.is_empty() && spec.name.len() <= 128 && names.insert(spec.name.clone()),
                "series names must be unique and 1..128 bytes"
            );
            ensure!(
                streams.contains(&spec.stream),
                "series stream '{}' is not subscribed",
                spec.stream
            );
            ensure!(
                spec.pattern.is_some() ^ spec.json_pointer.is_some(),
                "choose exactly one of pattern and json_pointer"
            );
            let regex = if let Some(pattern) = &spec.pattern {
                ensure!(pattern.len() <= 4096, "pattern exceeds 4096 bytes");
                let re = RegexBuilder::new(pattern)
                    .size_limit(2 * 1024 * 1024)
                    .dfa_size_limit(2 * 1024 * 1024)
                    .build()?;
                ensure!(
                    re.capture_names()
                        .flatten()
                        .any(|name| name == spec.capture),
                    "pattern is missing named capture '{}'",
                    spec.capture
                );
                Some(re)
            } else {
                let pointer = spec.json_pointer.as_ref().unwrap();
                ensure!(
                    pointer.len() <= 1024 && (pointer.is_empty() || pointer.starts_with('/')),
                    "invalid JSON pointer"
                );
                None
            };
            series.push(SeriesState {
                spec: spec.clone(),
                regex,
                points: VecDeque::new(),
                matched: 0,
                unmatched: 0,
                invalid: 0,
            });
        }
        Ok(Self {
            config,
            revision: 1,
            series,
            frozen: None,
        })
    }
}
impl PlotHub {
    pub fn new(config: OutputConfig) -> Result<Self> {
        let mut hub = Self {
            framers: config
                .streams
                .iter()
                .map(|s| (s.clone(), Framer::default()))
                .collect(),
            stats: config
                .streams
                .iter()
                .map(|s| (s.clone(), StreamStats::default()))
                .collect(),
            config: config.clone(),
            sessions: BTreeMap::new(),
            generation: 1,
        };
        for session in config.sessions {
            hub.create(session)?;
        }
        Ok(hub)
    }
    pub fn shared(config: OutputConfig) -> Result<SharedHub> {
        Ok(Arc::new(Mutex::new(Self::new(config)?)))
    }
    fn capacity(&self, replacement: Option<&str>, candidate: &SessionConfig) -> Result<()> {
        let allocated = self
            .sessions
            .iter()
            .filter(|(id, _)| Some(id.as_str()) != replacement)
            .map(|(_, s)| s.config.max_points * s.series.len())
            .sum::<usize>();
        ensure!(
            allocated + candidate.max_points * candidate.series.len() <= MAX_TOTAL_POINTS,
            "session point budget exceeds {MAX_TOTAL_POINTS}"
        );
        Ok(())
    }
    pub fn create(&mut self, config: SessionConfig) -> Result<Value> {
        ensure!(
            self.sessions.len() < MAX_SESSIONS,
            "at most {MAX_SESSIONS} sessions"
        );
        ensure!(
            !self.sessions.contains_key(&config.id),
            "session already exists"
        );
        let session = SessionState::new(config.clone(), &self.config.streams)?;
        self.capacity(None, &config)?;
        let id = config.id.clone();
        self.sessions.insert(id.clone(), session);
        self.generation += 1;
        self.summary(&id)
    }
    pub fn patch(&mut self, id: &str, revision: u64, patch: &Value) -> Result<Value> {
        let old = self.sessions.get(id).context("unknown session")?;
        ensure!(
            old.revision == revision,
            "revision_conflict: current revision is {}",
            old.revision
        );
        let changes = patch.as_object().context("patch must be an object")?;
        ensure!(!changes.contains_key("id"), "session id cannot change");
        let mut value = serde_json::to_value(&old.config)?;
        for (key, val) in changes {
            value
                .as_object_mut()
                .unwrap()
                .insert(key.clone(), val.clone());
        }
        let config: SessionConfig = serde_json::from_value(value)?;
        let mut new = SessionState::new(config.clone(), &self.config.streams)?;
        self.capacity(Some(id), &config)?;
        for target in &mut new.series {
            if let Some(source) = old.series.iter().find(|s| s.spec == target.spec) {
                target.points = source
                    .points
                    .iter()
                    .rev()
                    .take(config.max_points)
                    .cloned()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                target.matched = source.matched;
                target.unmatched = source.unmatched;
                target.invalid = source.invalid;
            }
        }
        new.revision = old.revision + 1;
        // Pausing freezes the visible point range; acquisition and bounded retention continue.
        if config.paused {
            let mut frozen = if old.config.paused && old.config.series == config.series {
                old.frozen
                    .clone()
                    .unwrap_or_else(|| build_snapshot(old, self.generation))
            } else {
                build_snapshot(&new, self.generation)
            };
            frozen.revision = new.revision;
            frozen.config = config.clone();
            frozen.paused = true;
            frozen.title = config.title.clone();
            frozen.theme = config.theme.clone();
            frozen.refresh_ms = config.refresh_ms;
            // A paused view keeps its captured samples. Other visual changes remain visible.
            apply_axis(&mut frozen, &config);
            frozen.source_status = frozen
                .config
                .series
                .iter()
                .filter_map(|s| {
                    self.stats
                        .get(&s.stream)
                        .map(|v| (s.stream.clone(), v.clone()))
                })
                .collect();
            new.frozen = Some(frozen);
        }
        self.sessions.insert(id.into(), new);
        self.generation += 1;
        self.summary(id)
    }
    pub fn summary(&self, id: &str) -> Result<Value> {
        let s = self.sessions.get(id).context("unknown session")?;
        Ok(
            json!({"id":id,"revision":s.revision,"config":s.config,"points":s.series.iter().map(|x|x.points.len()).sum::<usize>()}),
        )
    }
    pub fn summaries(&self) -> Value {
        json!({"sessions":self.sessions.keys().map(|id|self.summary(id).unwrap()).collect::<Vec<_>>(),"streams":self.stats,"generation":self.generation,
            "limits":{"max_sessions":MAX_SESSIONS,"max_series":MAX_SERIES,"total_points":MAX_TOTAL_POINTS,"view_points_per_series":VIEW_POINTS}})
    }
    pub fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }
    pub fn snapshot(&self, id: &str) -> Result<Snapshot> {
        let session = self.sessions.get(id).context("unknown session")?;
        let current_status: BTreeMap<String, StreamStats> = session
            .config
            .series
            .iter()
            .filter_map(|s| {
                self.stats
                    .get(&s.stream)
                    .map(|v| (s.stream.clone(), v.clone()))
            })
            .collect();
        let mut snapshot = if let Some(frozen) = &session.frozen {
            frozen.clone()
        } else {
            let mut view = build_snapshot(session, self.generation);
            view.source_status = current_status.clone();
            view
        };
        snapshot.generation = self.generation;
        // Pausing only freezes the plot; live transport failures remain visible.
        snapshot.live_source_status = current_status;
        Ok(snapshot)
    }
    pub fn ingest(&mut self, record: &Record) {
        if !self.framers.contains_key(&record.stream) {
            return;
        }
        let mut discontinuity = None;
        {
            let frame = self.framers.get(&record.stream).unwrap();
            if !frame.epoch.is_empty() && frame.epoch != record.epoch {
                discontinuity = Some((
                    record.seq,
                    "epoch changed; previous coverage is unknown".to_owned(),
                ));
            } else if frame.seq > 0 && record.seq <= frame.seq {
                self.stats
                    .get_mut(&record.stream)
                    .unwrap()
                    .duplicate_records += 1;
                return;
            } else if frame.seq > 0 && record.seq != frame.seq + 1 {
                discontinuity = Some((
                    frame.seq + 1,
                    format!("sequence jump {} -> {}", frame.seq, record.seq),
                ));
            }
        }
        if discontinuity
            .as_ref()
            .is_some_and(|(_, reason)| reason.starts_with("epoch changed"))
        {
            for session in self.sessions.values_mut() {
                for series in &mut session.series {
                    if series.spec.stream == record.stream {
                        series.points.clear();
                    }
                }
            }
        }
        if let Some((from, reason)) = discontinuity {
            self.gap(&record.stream, &record.epoch, from, record.seq, &reason);
        }
        let frame = self.framers.get_mut(&record.stream).unwrap();
        frame.epoch = record.epoch.clone();
        frame.seq = record.seq;
        let stat = self.stats.get_mut(&record.stream).unwrap();
        stat.records += 1;
        stat.disconnected = false;
        stat.disconnect_reason = None;
        stat.last_seq = record.seq;
        stat.epoch = record.epoch.clone();
        let source_x = record.observed_ts_ns as f64 / 1e9;
        if source_x < frame.last_x {
            stat.timestamp_regressions += 1;
        }
        // Core observation time is the view clock; clock regressions cannot reverse the display.
        let x = source_x.max(frame.last_x);
        frame.last_x = x;
        let mut lines = Vec::new();
        for &byte in &record.payload {
            if byte == b'\n' {
                if !frame.dropping {
                    if frame.bytes.last() == Some(&b'\r') {
                        frame.bytes.pop();
                    }
                    lines.push(std::mem::take(&mut frame.bytes));
                }
                frame.bytes.clear();
                frame.dropping = false;
            } else if !frame.dropping {
                if frame.bytes.len() == self.config.max_line_bytes {
                    frame.bytes.clear();
                    frame.dropping = true;
                    stat.oversized_lines += 1;
                } else {
                    frame.bytes.push(byte);
                }
            }
        }
        stat.pending_bytes = frame.bytes.len();
        for bytes in lines {
            stat.lines += 1;
            let Ok(text) = std::str::from_utf8(&bytes) else {
                stat.invalid_utf8 += 1;
                continue;
            };
            let mut parsed: Option<Result<Value, serde_json::Error>> = None;
            for session in self.sessions.values_mut() {
                for series in &mut session.series {
                    if series.spec.stream != record.stream {
                        continue;
                    }
                    let value = if let Some(regex) = &series.regex {
                        regex
                            .captures(text)
                            .and_then(|caps| caps.name(&series.spec.capture))
                            .map(|v| v.as_str().parse::<f64>().map_err(|_| ()))
                    } else {
                        let json = parsed.get_or_insert_with(|| serde_json::from_str(text));
                        match json {
                            Ok(value) => value
                                .pointer(series.spec.json_pointer.as_ref().unwrap())
                                .map(|v| v.as_f64().ok_or(())),
                            Err(_) => {
                                series.invalid += 1;
                                continue;
                            }
                        }
                    };
                    match value {
                        Some(Ok(y)) if y.is_finite() && y.abs() <= 1e100 => {
                            series.matched += 1;
                            series.points.push_back(Point {
                                x,
                                y: Some(y),
                                seq: record.seq,
                            });
                            while series.points.len() > session.config.max_points {
                                series.points.pop_front();
                            }
                            while series
                                .points
                                .front()
                                .is_some_and(|p| p.x < x - session.config.window_secs)
                            {
                                series.points.pop_front();
                            }
                        }
                        Some(_) => series.invalid += 1,
                        None => series.unmatched += 1,
                    }
                }
            }
        }
        self.generation += 1;
    }
    /// A lost transport is an interruption of unknown extent, not a measured record gap.
    pub fn disconnected(&mut self, stream: &str, reason: &str) {
        let Some(frame) = self.framers.get_mut(stream) else {
            return;
        };
        frame.bytes.clear();
        frame.dropping = false;
        let stats = self.stats.get_mut(stream).unwrap();
        stats.pending_bytes = 0;
        stats.disconnections += 1;
        stats.disconnected = true;
        stats.disconnect_reason = Some(reason.chars().take(256).collect());
        for session in self.sessions.values_mut() {
            for series in &mut session.series {
                if series.spec.stream == stream {
                    series.points.push_back(Point {
                        x: frame.last_x,
                        y: None,
                        seq: frame.seq,
                    });
                    while series.points.len() > session.config.max_points {
                        series.points.pop_front();
                    }
                }
            }
        }
        self.generation += 1;
    }
    pub fn gap(&mut self, stream: &str, epoch: &str, from: u64, to: u64, reason: &str) {
        let Some(frame) = self.framers.get_mut(stream) else {
            return;
        };
        frame.bytes.clear();
        frame.dropping = false;
        frame.seq = to.saturating_sub(1);
        frame.epoch = epoch.into();
        let stats = self.stats.get_mut(stream).unwrap();
        stats.gaps += 1;
        stats.pending_bytes = 0;
        stats.last_gap = Some(format!(
            "{from}..{to}: {}",
            reason.chars().take(256).collect::<String>()
        ));
        for session in self.sessions.values_mut() {
            for series in &mut session.series {
                if series.spec.stream == stream {
                    series.points.push_back(Point {
                        x: frame.last_x,
                        y: None,
                        seq: from,
                    });
                    while series.points.len() > session.config.max_points {
                        series.points.pop_front();
                    }
                }
            }
        }
        self.generation += 1;
    }
    pub fn export_request(&self, args: &Value) -> Result<ExportRequest> {
        let id = str_arg(args, "id")?;
        let snapshot = self.snapshot(id)?;
        if let Some(rev) = args.get("revision") {
            ensure!(
                rev.as_u64() == Some(snapshot.revision),
                "revision_conflict: current revision is {}",
                snapshot.revision
            );
        }
        let format = str_arg(args, "format")?.to_owned();
        let path = PathBuf::from(str_arg(args, "path")?);
        ensure!(
            path.extension().and_then(|s| s.to_str()) == Some(format.as_str()),
            "path extension must match format"
        );
        let width = args
            .get("width")
            .map(|v| v.as_u64().context("width must be an integer"))
            .transpose()?
            .unwrap_or(1200);
        let height = args
            .get("height")
            .map(|v| v.as_u64().context("height must be an integer"))
            .transpose()?
            .unwrap_or(700);
        ensure!(
            width <= 4096 && height <= 4096 && width >= 320 && height >= 240,
            "export size must be 320..4096 by 240..4096"
        );
        ensure!(width * height <= 8_388_608, "export exceeds 8 megapixels");
        ensure!(
            matches!(format.as_str(), "png" | "svg"),
            "format must be png or svg"
        );
        Ok(ExportRequest {
            snapshot,
            path,
            format,
            width: width as u32,
            height: height as u32,
            overwrite: args
                .get("overwrite")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}
fn str_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str> {
    args.get(name)
        .and_then(Value::as_str)
        .with_context(|| format!("missing string '{name}'"))
}
pub fn lock(hub: &SharedHub) -> Result<std::sync::MutexGuard<'_, PlotHub>> {
    hub.lock().map_err(|_| anyhow!("plot state poisoned"))
}
pub async fn control(hub: &SharedHub, method: &str, args: Value) -> Result<Value> {
    match method {
        "sessions" => Ok(lock(hub)?.summaries()),
        "config.get" => {
            let hub = lock(hub)?;
            let mut effective = serde_json::to_value(&hub.config)?;
            effective["sessions"] =
                serde_json::to_value(hub.sessions.values().map(|s| &s.config).collect::<Vec<_>>())?;
            Ok(
                json!({"effective":effective,"dynamic_fields":["session.title","session.series","session.window_secs","session.max_points","session.refresh_ms","session.theme","session.y_min","session.y_max","session.paused"],"restart_fields":["streams","from","max_line_bytes","headless","tty","web_bind"]}),
            )
        }
        "session.create" => lock(hub)?.create(serde_json::from_value(args)?),
        "session.patch" => {
            let id = str_arg(&args, "id")?;
            let revision = args
                .get("revision")
                .and_then(Value::as_u64)
                .context("revision is required")?;
            lock(hub)?.patch(
                id,
                revision,
                args.get("patch").context("patch is required")?,
            )
        }
        "session.get" => Ok(serde_json::to_value(
            lock(hub)?.snapshot(str_arg(&args, "id")?)?,
        )?),
        "session.export" => {
            let request = lock(hub)?.export_request(&args)?;
            tokio::task::spawn_blocking(move || request.write()).await?
        }
        _ => bail!("unknown control method: {method}"),
    }
}
fn build_snapshot(session: &SessionState, generation: u64) -> Snapshot {
    let end = session
        .series
        .iter()
        .filter_map(|s| s.points.back().map(|p| p.x))
        .reduce(f64::max)
        .unwrap_or(0.0);
    let start = end - session.config.window_secs;
    let series = session
        .series
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let points = s.points.iter().filter(|p| p.x >= start).collect::<Vec<_>>();
            SeriesSnapshot {
                name: s.spec.name.clone(),
                stream: s.spec.stream.clone(),
                color: COLORS[i].into(),
                data: decimate(&points),
                retained_points: points.len(),
                first_seq: points.first().map(|p| p.seq),
                last_seq: points.last().map(|p| p.seq),
                matched: s.matched,
                unmatched: s.unmatched,
                invalid: s.invalid,
            }
        })
        .collect();
    let mut out = Snapshot {
        id: session.config.id.clone(),
        revision: session.revision,
        generation,
        title: session.config.title.clone(),
        theme: session.config.theme.clone(),
        window_secs: session.config.window_secs,
        refresh_ms: session.config.refresh_ms,
        paused: session.config.paused,
        x_range: [start, end],
        y_range: [0.0, 1.0],
        series,
        config: session.config.clone(),
        exported_at_ns: log_proto::now_ns(),
        source_status: BTreeMap::new(),
        live_source_status: BTreeMap::new(),
    };
    apply_axis(&mut out, &session.config);
    out
}
fn apply_axis(view: &mut Snapshot, config: &SessionConfig) {
    let min = view
        .series
        .iter()
        .flat_map(|s| s.data.iter().filter_map(|p| p.1))
        .reduce(f64::min)
        .unwrap_or(0.0);
    let max = view
        .series
        .iter()
        .flat_map(|s| s.data.iter().filter_map(|p| p.1))
        .reduce(f64::max)
        .unwrap_or(1.0);
    let pad = ((max - min).abs() * 0.05).max(0.01).max(min.abs() * 1e-6);
    let mut low = config.y_min.unwrap_or(min - pad);
    let mut high = config.y_max.unwrap_or(max + pad);
    if low >= high {
        if config.y_min.is_some() {
            high = low + pad.max(1.0);
        } else {
            low = high - pad.max(1.0);
        }
    }
    // Guard finite extreme inputs whose padding could overflow.
    if !low.is_finite() {
        low = min;
    }
    if !high.is_finite() {
        high = max;
    }
    if low == high {
        low = 0.0;
        if high == 0.0 {
            high = 1.0;
        }
    }
    if low > high {
        std::mem::swap(&mut low, &mut high);
    }
    view.y_range = [low, high];
    let end = view.x_range[1];
    view.x_range = [end - config.window_secs, end];
    view.window_secs = config.window_secs;
}
fn decimate(points: &[&Point]) -> Vec<(f64, Option<f64>)> {
    if points.len() <= VIEW_POINTS {
        return points.iter().map(|p| (p.x, p.y)).collect();
    }
    let bucket = points.len().div_ceil(VIEW_POINTS / 3);
    let mut selected = Vec::new();
    for group in points.chunks(bucket) {
        // Include gaps as well as extrema; do not bridge unavailable intervals when reducing points.
        if group.iter().any(|p| p.y.is_none()) {
            selected.push((group[0].x, None));
            continue;
        }
        let min = group
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.y.unwrap().total_cmp(&b.y.unwrap()))
            .unwrap()
            .0;
        let max = group
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.y.unwrap().total_cmp(&b.y.unwrap()))
            .unwrap()
            .0;
        let mut indices = vec![min, max, group.len() - 1];
        indices.sort_unstable();
        indices.dedup();
        selected.extend(indices.into_iter().map(|i| (group[i].x, group[i].y)));
    }
    selected
}

pub struct ExportRequest {
    pub snapshot: Snapshot,
    pub path: PathBuf,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub overwrite: bool,
}
impl ExportRequest {
    pub fn write(self) -> Result<Value> {
        let bytes = encode(&self.snapshot, &self.format, self.width, self.height)?;
        let metadata = self.path.with_extension(format!("{}.json", self.format));
        let mut meta = serde_json::to_value(&self.snapshot)?;
        meta.as_object_mut().unwrap().insert("export".into(), json!({"format":self.format,"width":self.width,"height":self.height,
            "sampling":"same bounded extrema-preserving snapshot used by the view","font":"Noto Sans SC (SIL OFL 1.1)","view_points_per_series":VIEW_POINTS}));
        let metadata_bytes = serde_json::to_vec_pretty(&meta)?;
        if !self.overwrite {
            ensure!(
                !metadata.exists(),
                "metadata already exists: {}",
                metadata.display()
            );
        }
        write_file(&self.path, &bytes, self.overwrite)?;
        if let Err(error) = write_file(&metadata, &metadata_bytes, self.overwrite) {
            return Err(error.context(format!(
                "image exists at {} but metadata write failed",
                self.path.display()
            )));
        }
        Ok(
            json!({"path":self.path,"metadata_path":metadata,"revision":self.snapshot.revision,"generation":self.snapshot.generation,
            "bytes":bytes.len(),"width":self.width,"height":self.height,"x_range":self.snapshot.x_range,"y_range":self.snapshot.y_range}),
        )
    }
}
fn write_file(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true);
    if overwrite {
        options.create(true).truncate(true);
    } else {
        options.create_new(true);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn register_fonts() -> Result<()> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| {
            plotters::style::register_font(
                "noto",
                FontStyle::Normal,
                include_bytes!("../assets/NotoSansSC-Regular.otf"),
            )
            .map_err(|_| "font initialization failed".to_owned())
        })
        .clone()
        .map_err(anyhow::Error::msg)
}
pub fn encode(snapshot: &Snapshot, format: &str, width: u32, height: u32) -> Result<Vec<u8>> {
    ensure!(
        (320..=4096).contains(&width)
            && (240..=4096).contains(&height)
            && width as u64 * height as u64 <= 8_388_608,
        "invalid export dimensions"
    );
    register_fonts()?;
    match format {
        "svg" => {
            let mut svg = String::new();
            {
                let area = SVGBackend::with_string(&mut svg, (width, height)).into_drawing_area();
                draw(snapshot, &area)?;
                area.present()
                    .map_err(|e| anyhow!("SVG completion: {e:?}"))?;
            }
            // Keep the export self contained, including non-Latin labels, when moved to another system.
            // Plotters SVG names the same font family; CSS embeds the exact font used by PNG.
            let font = base64::engine::general_purpose::STANDARD
                .encode(include_bytes!("../assets/NotoSansSC-Regular.otf"));
            let style = format!("<style>@font-face{{font-family:noto;src:url(data:font/otf;base64,{font}) format('opentype')}}</style>");
            if let Some(position) = svg.find('>') {
                svg.insert_str(position + 1, &style);
            }
            Ok(svg.into_bytes())
        }
        "png" => {
            let mut rgb = vec![0u8; width as usize * height as usize * 3];
            {
                let area =
                    BitMapBackend::with_buffer(&mut rgb, (width, height)).into_drawing_area();
                draw(snapshot, &area)?;
                area.present().map_err(|e| anyhow!("PNG drawing: {e:?}"))?;
            }
            let mut bytes = Vec::new();
            image::ImageEncoder::write_image(
                image::codecs::png::PngEncoder::new(&mut bytes),
                &rgb,
                width,
                height,
                image::ColorType::Rgb8,
            )?;
            Ok(bytes)
        }
        _ => bail!("format must be png or svg"),
    }
}
fn draw<DB: DrawingBackend>(
    snapshot: &Snapshot,
    area: &DrawingArea<DB, plotters::coord::Shift>,
) -> Result<()> {
    let dark = snapshot.theme == "dark";
    let bg = if dark {
        RGBColor(20, 27, 39)
    } else {
        RGBColor(250, 251, 253)
    };
    let fg = if dark {
        RGBColor(222, 229, 238)
    } else {
        RGBColor(30, 40, 55)
    };
    let grid = if dark {
        RGBColor(54, 65, 82)
    } else {
        RGBColor(218, 225, 234)
    };
    area.fill(&bg)
        .map_err(|e| anyhow!("chart background: {e:?}"))?;
    let offset = snapshot.x_range[0];
    let duration = snapshot.x_range[1] - offset;
    let mut chart = ChartBuilder::on(area)
        .margin(24)
        .caption(
            format!(
                "{} · {} · revision {}",
                snapshot.title, snapshot.id, snapshot.revision
            ),
            ("noto", 24).into_font().color(&fg),
        )
        .x_label_area_size(50)
        .y_label_area_size(70)
        .build_cartesian_2d(0f64..duration, snapshot.y_range[0]..snapshot.y_range[1])
        .map_err(|e| anyhow!("chart axes: {e:?}"))?;
    chart
        .configure_mesh()
        .x_labels(6)
        .y_labels(6)
        .max_light_lines(0)
        .axis_style(fg)
        .light_line_style(grid)
        .bold_line_style(grid)
        .label_style(("noto", 14).into_font().color(&fg))
        .axis_desc_style(("noto", 16).into_font().color(&fg))
        .x_desc("Time in selected window (seconds)")
        .y_desc("Value")
        .draw()
        .map_err(|e| anyhow!("chart mesh: {e:?}"))?;
    for (index, series) in snapshot.series.iter().enumerate() {
        let color = parse_color(COLORS[index]);
        let mut first = true;
        for segment in series.data.split(|(_, y)| y.is_none()) {
            if segment.is_empty() {
                continue;
            }
            let points = segment
                .iter()
                .filter_map(|(x, y)| y.map(|y| (*x - offset, y)))
                .collect::<Vec<_>>();
            let annotation = chart
                .draw_series(LineSeries::new(points.clone(), color.stroke_width(2)))
                .map_err(|e| anyhow!("chart series: {e:?}"))?;
            if first {
                annotation.label(series.name.clone()).legend(move |(x, y)| {
                    PathElement::new([(x, y), (x + 24, y)], color.stroke_width(2))
                });
                first = false;
            }
            if points.len() == 1 {
                chart
                    .draw_series(
                        points
                            .into_iter()
                            .map(|p| Circle::new(p, 3, color.filled())),
                    )
                    .map_err(|e| anyhow!("chart point: {e:?}"))?;
            }
        }
    }
    chart
        .configure_series_labels()
        .background_style(bg.mix(0.9))
        .border_style(grid)
        .label_font(("noto", 14).into_font().color(&fg))
        .draw()
        .map_err(|e| anyhow!("chart legend: {e:?}"))?;
    Ok(())
}
fn parse_color(value: &str) -> RGBColor {
    RGBColor(
        u8::from_str_radix(&value[1..3], 16).unwrap(),
        u8::from_str_radix(&value[3..5], 16).unwrap(),
        u8::from_str_radix(&value[5..7], 16).unwrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hub() -> PlotHub {
        PlotHub::new(OutputConfig::from_value(&json!({"streams":["sensor"],"sessions":[{"id":"a","series":[{"name":"temperature","stream":"sensor","pattern":"temp=(?P<value>[-0-9.]+)"}]}]})).unwrap()).unwrap()
    }
    fn record(seq: u64, payload: &[u8]) -> Record {
        Record {
            stream: "sensor".into(),
            epoch: "e1".into(),
            seq,
            key: seq.to_string(),
            payload: payload.into(),
            source_ts_ns: None,
            observed_ts_ns: 1_000_000_000 + seq * 1_000_000,
            upstream: BTreeMap::new(),
            upstream_epochs: BTreeMap::new(),
            durability: "buffered".into(),
        }
    }
    #[test]
    fn chunk_boundaries_and_session_isolation() {
        let mut h = hub();
        h.ingest(&record(1, b"temp=1"));
        h.ingest(&record(2, b"2.5\ntemp=-2\n"));
        assert_eq!(
            h.snapshot("a").unwrap().series[0]
                .data
                .iter()
                .map(|p| p.1)
                .collect::<Vec<_>>(),
            vec![Some(12.5), Some(-2.0)]
        );
        let mut other = h.sessions["a"].config.clone();
        other.id = "b".into();
        h.create(other).unwrap();
        h.patch("a", 1, &json!({"title":"changed"})).unwrap();
        assert_eq!(h.snapshot("b").unwrap().revision, 1);
        assert!(h.patch("a", 1, &json!({"title":"stale"})).is_err());
    }
    #[test]
    fn gaps_clear_partial_and_duplicate_is_ignored() {
        let mut h = hub();
        h.ingest(&record(1, b"temp=4\ntemp=1"));
        h.gap("sensor", "e1", 2, 3, "test");
        h.ingest(&record(3, b"7\ntemp=5\n"));
        h.ingest(&record(3, b"temp=9\n"));
        let s = h.snapshot("a").unwrap();
        assert_eq!(s.series[0].matched, 2);
        assert!(s.series[0].data.iter().any(|p| p.1.is_none()));
    }
    #[test]
    fn invalid_patch_is_atomic_and_bounded() {
        let mut h = hub();
        assert!(h
            .patch(
                "a",
                1,
                &json!({"series":[{"name":"x","stream":"sensor","pattern":"("}]})
            )
            .is_err());
        assert_eq!(h.snapshot("a").unwrap().revision, 1);
        h.patch("a", 1, &json!({"max_points":16})).unwrap();
        for n in 1..100 {
            h.ingest(&record(n, format!("temp={n}\n").as_bytes()));
        }
        assert_eq!(h.snapshot("a").unwrap().series[0].retained_points, 16);
    }
    #[test]
    fn frozen_view_keeps_samples_while_ingest_continues() {
        let mut h = hub();
        h.ingest(&record(1, b"temp=10\n"));
        h.patch("a", 1, &json!({"paused":true})).unwrap();
        h.ingest(&record(2, b"temp=20\n"));
        assert_eq!(h.snapshot("a").unwrap().series[0].data.len(), 1);
        h.patch("a", 2, &json!({"paused":false})).unwrap();
        assert_eq!(h.snapshot("a").unwrap().series[0].data.len(), 2);
    }
    #[test]
    fn paused_plot_still_reports_unknown_disconnect_extent() {
        let mut h = hub();
        h.ingest(&record(1, b"temp=12\n"));
        h.patch("a", 1, &json!({"paused":true})).unwrap();
        h.disconnected("sensor", "test transport failure");
        let view = h.snapshot("a").unwrap();
        assert_eq!(view.series[0].data.len(), 1);
        assert!(view.live_source_status["sensor"].disconnected);
        assert_eq!(view.live_source_status["sensor"].gaps, 0);
        assert_eq!(view.live_source_status["sensor"].disconnections, 1);
        assert!(!view.source_status["sensor"].disconnected);
    }
    #[test]
    fn json_pointer_and_oversized_invalid_lines_are_observable() {
        let cfg=OutputConfig::from_value(&json!({"streams":["sensor"],"max_line_bytes":256,
            "sessions":[{"id":"json","series":[{"name":"v","stream":"sensor","json_pointer":"/value"}]}]})).unwrap();
        let mut h = PlotHub::new(cfg).unwrap();
        h.ingest(&record(1, b"{\"value\":1"));
        h.ingest(&record(2, b"2.5}\n{\"value\":null}\n"));
        h.ingest(&record(3, &vec![b'x'; 300]));
        h.ingest(&record(4, b"\n\xff\n"));
        let v = h.snapshot("json").unwrap();
        assert_eq!(v.series[0].data[0].1, Some(12.5));
        assert_eq!(v.series[0].invalid, 1);
        assert_eq!(v.source_status["sensor"].oversized_lines, 1);
        assert_eq!(v.source_status["sensor"].invalid_utf8, 1);
        assert_eq!(v.source_status["sensor"].pending_bytes, 0);
    }
    #[test]
    fn exports_decode_with_embedded_font() {
        let mut h = hub();
        h.ingest(&record(1, b"temp=12\ntemp=16\n"));
        let s = h.snapshot("a").unwrap();
        let png = encode(&s, "png", 640, 400).unwrap();
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!(decoded.width(), 640);
        assert_eq!(decoded.height(), 400);
        let svg = String::from_utf8(encode(&s, "svg", 640, 400).unwrap()).unwrap();
        assert!(svg.contains("<svg"));
        assert!(svg.contains("data:font/otf;base64,"));
    }
}
