use anyhow::{bail, Result};
use io_plugin_util::stopped;
use log_plugin_sdk::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::io::{AsyncWrite, AsyncWriteExt};
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    streams: Vec<String>,
    path: Option<PathBuf>,
    paths: BTreeMap<String, PathBuf>,
    from: u64,
    append: bool,
    overwrite: bool,
    fail_on_gap: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            streams: vec![],
            path: None,
            paths: BTreeMap::new(),
            from: 1,
            append: false,
            overwrite: false,
            fail_on_gap: true,
        }
    }
}
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    if c.streams.is_empty() {
        bail!("nonempty streams required")
    };
    if c.path.is_some() && !c.paths.is_empty() {
        bail!("path and paths are mutually exclusive")
    };
    if !c.paths.is_empty() && c.streams.iter().any(|s| !c.paths.contains_key(s)) {
        bail!("paths must contain every subscribed stream")
    };
    if c.paths
        .values()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != c.paths.len()
    {
        bail!("per-stream output paths must be distinct")
    };
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
}
async fn writer(path: Option<&PathBuf>, c: &Config) -> Result<Box<dyn AsyncWrite + Unpin + Send>> {
    if let Some(path) = path {
        let mut o = tokio::fs::OpenOptions::new();
        o.write(true);
        if c.append {
            o.create(true).append(true);
        } else if c.overwrite {
            o.create(true).truncate(true);
        } else {
            o.create_new(true);
        }
        Ok(Box::new(o.open(path).await?))
    } else {
        Ok(Box::new(tokio::io::stdout()))
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut cx = io_plugin_util::connect(validate, &[]).await?;
    let client = cx.client.clone();
    let r = run(&mut cx).await;
    io_plugin_util::finish(&client, &r).await;
    r
}
async fn run(cx: &mut io_plugin_util::Context) -> Result<()> {
    let c = config(&cx.config.borrow())?;
    let mut outputs = BTreeMap::new();
    if c.paths.is_empty() {
        outputs.insert(String::new(), writer(c.path.as_ref(), &c).await?);
    } else {
        for (s, path) in &c.paths {
            outputs.insert(s.clone(), writer(Some(path), &c).await?);
        }
    }
    for stream in &c.streams {
        cx.client.subscribe(stream, c.from).await?;
    }
    cx.client.request("report",json!({"state":"outputting","streams":c.streams,"destination":c.path,"byte_preserving":true,"multi_stream_order":"arrival_order_no_global_order"})).await?;
    loop {
        let event = tokio::select! {_=stopped(&mut cx.shutdown)=>{for out in outputs.values_mut(){out.flush().await?;}return Ok(())},event=cx.events.recv()=>event.ok_or_else(||anyhow::anyhow!("event connection closed"))?};
        match event {
            Event::Record(r) => {
                let key = if c.paths.is_empty() {
                    ""
                } else {
                    r.stream.as_str()
                };
                let output = outputs
                    .get_mut(key)
                    .ok_or_else(|| anyhow::anyhow!("unexpected stream {}", r.stream))?;
                tokio::select! {
                    _=stopped(&mut cx.shutdown)=>{eprintln!("shutdown during raw write; current record may be partially written");return Ok(())},
                    result=async {output.write_all(&r.payload).await?;output.flush().await}=>result?,
                }
            }
            Event::Disconnected { stream, reason } => {
                bail!("event connection lost for {stream}: {reason}; output completeness unknown")
            }
            Event::Gap {
                stream,
                epoch,
                from,
                to,
                reason,
            } => {
                eprintln!("gap {stream}/{epoch} from={from} through={to}: {reason}");
                cx.client.request("report",json!({"state":"gap","stream":stream,"epoch":epoch,"from":from,"to":to,"reason":reason})).await?;
                if c.fail_on_gap {
                    bail!("raw output stopped on gap; output prefix may be incomplete")
                }
            }
        }
    }
}
