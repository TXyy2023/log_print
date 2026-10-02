use anyhow::{bail, Context, Result};
use fs2::FileExt;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{fs::File, path::Path};

pub struct Store {
    db: Connection,
    _lock: File,
    pub state: Value,
}
fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn page(name: &str) -> Value {
    json!({"id":id(),"name":name,"title":name,"theme":"dark","order":0,"panels":[]})
}
fn merge(target: &mut Value, args: &Value, fields: &[&str]) {
    for field in fields {
        if let Some(v) = args.get(*field) {
            target[*field] = v.clone();
        }
    }
}
fn locate(list: &[Value], key: &str) -> Result<usize> {
    list.iter()
        .position(|v| v["id"] == key || v["name"] == key)
        .context("not found")
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path.parent().context("state directory")?)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        lock.try_lock_exclusive()
            .context("Page database is in use by another WebUI")?;
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(1))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS configuration(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL, body TEXT NOT NULL)")?;
        let initial = page("overview");
        let state = json!({"revision":0,"selected":initial["id"],"pages":[initial]});
        db.execute(
            "INSERT OR IGNORE INTO configuration VALUES(1,0,?1)",
            [serde_json::to_string(&state)?],
        )?;
        let body: String = db.query_row("SELECT body FROM configuration WHERE id=1", [], |r| {
            r.get(0)
        })?;
        let mut state: Value = serde_json::from_str(&body)?;
        let mut reset = false;
        for page in state["pages"].as_array_mut().context("pages")? {
            for panel in page["panels"].as_array_mut().context("panels")? {
                if panel["query"].is_string() {
                    panel["query"] = Value::Null;
                    panel["mode"] = json!("live");
                    panel["offset"] = json!(0);
                    reset = true;
                }
            }
        }
        if reset {
            state["revision"] = json!(state["revision"].as_u64().context("revision")? + 1);
            db.execute(
                "UPDATE configuration SET revision=?1,body=?2 WHERE id=1",
                rusqlite::params![state["revision"].as_u64(), serde_json::to_string(&state)?],
            )?;
        }

        validate(&state)?;
        Ok(Self {
            db,
            _lock: lock,
            state,
        })
    }
    pub fn command(&mut self, method: &str, args: &Value) -> Result<Value> {
        if method == "page.list" {
            return Ok(
                json!({"revision":self.state["revision"],"selected":self.state["selected"],"pages":self.state["pages"].as_array().unwrap().iter().map(|p|json!({"id":p["id"],"name":p["name"],"title":p["title"],"theme":p["theme"],"order":p["order"],"panel_count":p["panels"].as_array().unwrap().len()})).collect::<Vec<_>>()}),
            );
        }
        if method == "state.get" {
            return Ok(self.state.clone());
        }
        let mut next = self.state.clone();
        if let Some(revision) = args["revision"].as_u64() {
            if revision != self.state["revision"].as_u64().unwrap_or(0) {
                bail!("revision_conflict: reload state before editing");
            }
        }
        let mut result = Value::Null;
        if method == "page.create" {
            let name = args["name"].as_str().context("name required")?;
            let mut p = page(name);
            merge(&mut p, args, &["title", "theme", "order"]);
            result = p.clone();
            next["pages"].as_array_mut().unwrap().push(p);
        } else {
            let key = args["page"]
                .as_str()
                .or_else(|| next["selected"].as_str())
                .context("page required")?
                .to_owned();
            let pages = next["pages"].as_array_mut().context("pages")?;
            let i = locate(pages, &key)?;
            match method {
                "page.get" => {
                    return Ok(json!({"revision":self.state["revision"],"page":pages[i]}))
                }
                "page.set" => merge(&mut pages[i], args, &["name", "title", "theme", "order"]),
                "page.clone" => {
                    let mut p = pages[i].clone();
                    p["id"] = json!(id());
                    p["name"] = args.get("name").cloned().context("new name required")?;
                    merge(&mut p, args, &["title", "order"]);
                    for panel in p["panels"].as_array_mut().unwrap() {
                        panel["id"] = json!(id());
                        for series in panel["series"].as_array_mut().unwrap() {
                            series["id"] = json!(id());
                        }
                    }
                    result = p.clone();
                    pages.push(p);
                }
                "page.delete" => {
                    if pages.len() == 1 {
                        bail!("at least one Page is required");
                    }
                    let removed = pages.remove(i);
                    if next["selected"] == removed["id"] {
                        next["selected"] = next["pages"][0]["id"].clone();
                    }
                }
                "page.select" => next["selected"] = pages[i]["id"].clone(),
                "panel.add" => {
                    let mut p = json!({"id":id(),"kind":"log","title":"Logs","x":0,"y":0,"w":12,"h":6,"streams":[],"channels":[],"text":"","regex":"","format":"text","columns":["time","stream","channel","text"],"metadata":false,"follow":true,"paused":false,"legend":true,"mode":"live","series":[]});
                    merge(&mut p, args, PANEL_FIELDS);
                    column_settings(&mut p, args)?;
                    result = p.clone();
                    pages[i]["panels"].as_array_mut().unwrap().push(p);
                }
                "panel.get" | "panel.set" | "panel.remove" | "series.add" | "series.set"
                | "series.remove" => {
                    let panels = pages[i]["panels"].as_array_mut().unwrap();
                    let j = locate(panels, args["panel"].as_str().context("panel required")?)?;
                    match method {
                        "panel.get" => {
                            return Ok(json!({"revision":self.state["revision"],"panel":panels[j]}))
                        }
                        "panel.set" => {
                            merge(&mut panels[j], args, PANEL_FIELDS);
                            column_settings(&mut panels[j], args)?;
                        }
                        "panel.remove" => {
                            panels.remove(j);
                        }
                        "series.add" => {
                            let mut s = json!({"id":id(),"name":"value","streams":[],"regex":"(?P<value>-?[0-9]+(?:\\.[0-9]+)?)","field":"","color":"#50c8b8","width":2});
                            merge(&mut s, args, SERIES_FIELDS);
                            result = s.clone();
                            panels[j]["series"].as_array_mut().unwrap().push(s);
                        }
                        _ => {
                            let series = panels[j]["series"].as_array_mut().unwrap();
                            let k = locate(
                                series,
                                args["series"].as_str().context("series required")?,
                            )?;
                            if method == "series.remove" {
                                series.remove(k);
                            } else {
                                merge(&mut series[k], args, SERIES_FIELDS);
                            }
                        }
                    }
                }
                "layout.set" => {
                    for item in args["layout"].as_array().context("layout required")? {
                        let panels = pages[i]["panels"].as_array_mut().unwrap();
                        let j = locate(panels, item["id"].as_str().context("panel id")?)?;
                        merge(&mut panels[j], item, &["x", "y", "w", "h"]);
                    }
                }
                _ => bail!("unsupported control {method}"),
            }
        }
        next["revision"] = json!(self.state["revision"]
            .as_u64()
            .context("revision")?
            .checked_add(1)
            .context("revision exhausted")?);
        validate(&next)?;
        let body = serde_json::to_string(&next)?;
        if body.len() > 512 * 1024 {
            bail!("Page configuration exceeds 512 KiB");
        }
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE configuration SET revision=?1,body=?2 WHERE id=1",
            rusqlite::params![next["revision"].as_u64(), body],
        )?;
        tx.commit()?;
        self.state = next;
        Ok(json!({"revision":self.state["revision"],"result":result,"state":self.state}))
    }
}
const PANEL_FIELDS: &[&str] = &[
    "title",
    "kind",
    "x",
    "y",
    "w",
    "h",
    "streams",
    "channels",
    "text",
    "regex",
    "format",
    "columns",
    "column_state",
    "metadata",
    "follow",
    "paused",
    "legend",
    "legend_selected",
    "mode",
    "query",
    "offset",
    "time_from",
    "time_end",
    "y_min",
    "y_max",
    "zoom_start",
    "zoom_end",
];
const SERIES_FIELDS: &[&str] = &[
    "name", "streams", "channels", "regex", "field", "color", "width",
];
fn regex(value: &Value) -> Result<()> {
    let text = value.as_str().context("regex must be text")?;
    if text.len() > 4096 {
        bail!("regex too long");
    }
    if !text.is_empty() {
        regex::RegexBuilder::new(text)
            .size_limit(2 * 1024 * 1024)
            .build()?;
    }
    Ok(())
}
fn validate(state: &Value) -> Result<()> {
    let pages = state["pages"].as_array().context("pages")?;
    if pages.is_empty() || pages.len() > 64 {
        bail!("Page limit is 1..64");
    }
    let mut names = std::collections::BTreeSet::new();
    for page in pages {
        let name = page["name"].as_str().context("page name")?;
        if name.is_empty() || name.len() > 128 || !names.insert(name) {
            bail!("Page name must be unique and 1..128 bytes");
        }
        if !matches!(page["theme"].as_str(), Some("dark" | "light")) {
            bail!("theme must be dark or light");
        }
        let panels = page["panels"].as_array().context("panels")?;
        if panels.len() > 32 {
            bail!("at most 32 panels per Page");
        }
        for p in panels {
            if !matches!(p["kind"].as_str(), Some("log" | "curve")) {
                bail!("panel kind must be log or curve");
            }
            let x = p["x"].as_u64().context("x")?;
            let w = p["w"].as_u64().context("w")?;
            let h = p["h"].as_u64().context("h")?;
            if w == 0
                || x.checked_add(w).is_none_or(|end| end > 12)
                || h == 0
                || h > 100
                || p["y"].as_u64().is_none_or(|v| v > 10000)
            {
                bail!("invalid 12-column layout");
            }
            regex(&p["regex"])?;
            if !matches!(p["format"].as_str(), Some("text" | "hex"))
                || !matches!(p["mode"].as_str(), Some("live" | "history"))
            {
                bail!("invalid format or mode");
            }
            for key in ["streams", "channels", "columns", "series"] {
                if p[key].as_array().is_none_or(|v| v.len() > 128) {
                    bail!("invalid {key}");
                }
            }
            for key in ["paused", "follow", "legend", "metadata"] {
                if !p[key].is_boolean() {
                    bail!("{key} must be boolean");
                }
            }
            for s in p["series"].as_array().unwrap() {
                regex(&s["regex"])?;
                if s["field"].as_str().context("JSON field")?.is_empty()
                    && !regex::Regex::new(s["regex"].as_str().unwrap())?
                        .capture_names()
                        .any(|n| n == Some("value"))
                {
                    bail!("series regex requires a named value group");
                }
                if s["width"]
                    .as_f64()
                    .is_none_or(|v| !(0.1..=20.).contains(&v))
                {
                    bail!("line width must be 0.1..20");
                }
            }
        }
    }
    Ok(())
}

fn column_settings(panel: &mut Value, args: &Value) -> Result<()> {
    if args.get("column_state").is_some() {
        return Ok(());
    }
    if args.get("columns").is_none()
        && args.get("column_width").is_none()
        && args.get("sort_column").is_none()
    {
        return Ok(());
    }
    let columns = panel["columns"].as_array().context("columns")?;
    let fields = [
        "time",
        "stream",
        "channel",
        "seq",
        "offset",
        "key",
        "text",
        "epoch",
        "source_ts_ns",
        "source_seq",
        "upstream",
        "upstream_epochs",
    ];
    let mut state = panel["column_state"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for field in fields {
        if !state.iter().any(|s| s["colId"] == field) {
            state.push(json!({"colId":field,"width":if field=="text"{500}else{140},"hide":!columns.iter().any(|c|c==field)}));
        }
    }
    if args.get("columns").is_some() {
        state.sort_by_key(|s| {
            columns
                .iter()
                .position(|c| c == &s["colId"])
                .unwrap_or(usize::MAX)
        });
        for item in &mut state {
            item["hide"] = json!(!columns.iter().any(|c| c == &item["colId"]));
        }
    }
    for item in &mut state {
        let field = item["colId"].as_str().context("colId")?.to_owned();
        if let Some(width) = args["column_width"].get(&field) {
            let width = width.as_u64().context("column width")?;
            if !(40..=4000).contains(&width) {
                bail!("column width must be 40..4000");
            }
            item["width"] = json!(width);
            item["flex"] = Value::Null;
        }
        if let Some(column) = args["sort_column"].as_str() {
            let order = args["sort_order"].as_str().unwrap_or("asc");
            if !["asc", "desc"].contains(&order) {
                bail!("sort order must be asc or desc");
            }
            item["sort"] = if column == field {
                json!(order)
            } else {
                Value::Null
            };
        }
    }
    panel["column_state"] = json!(state);
    Ok(())
}
