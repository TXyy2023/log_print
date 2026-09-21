use anyhow::{bail, Result};
use log_plugin_sdk::{stopped, Event};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use tokio::io::AsyncWriteExt;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    streams: Vec<String>,
    /// A metadata line before each record. Payload bytes remain unchanged.
    #[serde(default)]
    annotate: bool,
}
fn config(value: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(value.clone())?;
    if c.streams.iter().any(|s| s.is_empty())
        || c.streams.iter().collect::<BTreeSet<_>>().len() != c.streams.len()
    {
        bail!("streams must be nonempty, unique names or IDs");
    }
    Ok(c)
}
fn validate(value: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(value)?)?)
}
fn display(record: &log_proto::Record, annotate: bool) -> Vec<u8> {
    let mut bytes = if annotate {
        format!(
            "\n[{} #{} channel={}]\n",
            record.stream,
            record.seq,
            record.channel.as_deref().unwrap_or("default")
        )
        .into_bytes()
    } else {
        Vec::new()
    };
    bytes.extend_from_slice(&record.payload);
    bytes
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut cx = log_plugin_sdk::connect(validate).await?;
    let result = run(&mut cx).await.and_then(|_| cx.shutdown_result());
    log_plugin_sdk::finish(&cx.client, &result).await;
    result
}
async fn run(cx: &mut log_plugin_sdk::Context) -> Result<()> {
    let mut c = config(&cx.config)?;
    if !cx.client.read_streams().is_empty() {
        c.streams = cx.client.read_streams().to_vec();
    }
    if c.streams.is_empty() {
        bail!("configure or attach at least one source stream");
    }
    for stream in &c.streams {
        cx.client.subscribe(stream).await?;
    }
    cx.client.request("report",json!({"state":"displaying","streams":c.streams,"destination":"stdout","annotate":c.annotate})).await?;
    let mut output = tokio::io::stdout();
    loop {
        let event = tokio::select! {_=stopped(&mut cx.shutdown)=>break,event=cx.events.recv()=>event.ok_or_else(||anyhow::anyhow!("event channel closed"))?};
        match event {
            Event::Record(record) => {
                output.write_all(&display(&record, c.annotate)).await?;
                output.flush().await?;
            }
            Event::Disconnected { stream, reason } => {
                bail!("display disconnected for {stream}: {reason}")
            }
            Event::Gap {
                stream, from, to, ..
            } => eprintln!("display missing records {stream} {from}..={to}"),
        }
    }
    output.flush().await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_file_output_and_invalid_streams() {
        for value in [
            json!({"streams":["s"],"path":"data"}),
            json!({"streams":[""]}),
            json!({"streams":["s","s"]}),
        ] {
            assert!(config(&value).is_err());
        }
    }
    #[test]
    fn default_display_preserves_binary_and_empty_records() {
        let mut r:log_proto::Record=serde_json::from_value(json!({"stream":"s","epoch":"e","seq":2,"key":"k","payload":[0,255,10],"observed_ts_ns":1,"upstream":{},"upstream_epochs":{}})).unwrap();
        assert_eq!(display(&r, false), vec![0, 255, 10]);
        assert!(display(&r, true).ends_with(&r.payload));
        r.payload.clear();
        assert_eq!(display(&r, false), Vec::<u8>::new());
    }
}
