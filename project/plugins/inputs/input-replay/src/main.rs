use anyhow::{bail, Context as _, Result};
use io_plugin_util::{bounded, stopped};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio::io::{AsyncBufReadExt, BufReader};
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    path: PathBuf,
    stream: String,
    chunk_bytes: usize,
    interval_ms: u64,
    speed: f64,
    timestamp: String,
    max_gap_ms: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            stream: "replay".into(),
            chunk_bytes: 4096,
            interval_ms: 10,
            speed: 1.0,
            timestamp: "auto".into(),
            max_gap_ms: 60000,
        }
    }
}
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    if c.path.as_os_str().is_empty() || c.stream.is_empty() {
        bail!("path and stream required")
    };
    bounded("chunk_bytes", c.chunk_bytes, 1, log_proto::MAX_PAYLOAD)?;
    bounded("interval_ms", c.interval_ms as usize, 0, 60000)?;
    bounded("max_gap_ms", c.max_gap_ms as usize, 1, 3600000)?;
    if !c.speed.is_finite() || !(0.01..=1000.).contains(&c.speed) {
        bail!("speed must be 0.01..=1000")
    };
    if !["auto", "none", "rfc3339", "unix_ms", "unix_ns"].contains(&c.timestamp.as_str()) {
        bail!("invalid timestamp mode")
    };
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
}
fn timestamp(bytes: &[u8], mode: &str) -> Option<u64> {
    if mode == "none" {
        return None;
    };
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if mode == "auto" {
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            if let Some(ns) = v.get("source_ts_ns").and_then(Value::as_u64) {
                return Some(ns);
            };
            if let Some(s) = v.get("timestamp").and_then(Value::as_str) {
                return timestamp(s.as_bytes(), "rfc3339");
            }
        }
    }
    let token = text
        .split_ascii_whitespace()
        .next()?
        .trim_matches(|c| c == '[' || c == ']');
    match mode {
        "unix_ns" => token.parse().ok(),
        "unix_ms" => token.parse::<u64>().ok()?.checked_mul(1000000),
        _ => {
            let d = chrono::DateTime::parse_from_rfc3339(token).ok()?;
            u64::try_from(d.timestamp_nanos_opt()?).ok()
        }
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut cx = io_plugin_util::connect(validate, &["speed", "interval_ms"]).await?;
    let client = cx.client.clone();
    let r = run(&mut cx).await;
    io_plugin_util::finish(&client, &r).await;
    r
}
async fn run(cx: &mut io_plugin_util::Context) -> Result<()> {
    let c = config(&cx.config.borrow())?;
    let mut file = BufReader::with_capacity(
        c.chunk_bytes,
        tokio::fs::File::open(&c.path)
            .await
            .with_context(|| format!("open {}", c.path.display()))?,
    );
    let run = uuid::Uuid::new_v4();
    let mut offset = 0u64;
    let mut previous = None;
    let mut line_start = true;
    let mut source_count = 0u64;
    let mut simulated_count = 0u64;
    cx.client.request("report",json!({"state":"replaying","run":run,"path":c.path,"timestamp_mode":c.timestamp,"pace_without_timestamp":"simulated","payload":"unchanged"})).await?;
    loop {
        let chunk =
            tokio::select! {_=stopped(&mut cx.shutdown)=>return Ok(()),r=file.fill_buf()=>r?};
        if chunk.is_empty() {
            break;
        }
        let n = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(chunk.len())
            .min(c.chunk_bytes);
        let payload = chunk[..n].to_vec();
        file.consume(n);
        let prefix_complete = payload.iter().any(u8::is_ascii_whitespace)
            || serde_json::from_slice::<Value>(&payload).is_ok();
        let ts = if line_start && prefix_complete {
            timestamp(&payload, &c.timestamp)
        } else {
            None
        };
        let ends_line = payload.last() == Some(&b'\n');
        let tuning = config(&cx.config.borrow())?;
        let delay_ns = if offset == 0 {
            0
        } else if let (Some(now), Some(last)) = (ts, previous) {
            if now >= last {
                now - last
            } else {
                cx.client
                    .request(
                        "report",
                        json!({"state":"timestamp_regressed","offset":offset,"pace":"simulated"}),
                    )
                    .await?;
                tuning.interval_ms * 1000000
            }
        } else if line_start {
            tuning.interval_ms * 1000000
        } else {
            0
        };
        let delay = Duration::from_secs_f64(
            (delay_ns as f64 / 1e9 / tuning.speed).min(c.max_gap_ms as f64 / 1000.),
        );
        if delay_ns as f64 / 1e6 / tuning.speed > c.max_gap_ms as f64 {
            cx.client
                .request(
                    "report",
                    json!({"state":"replay_gap_clamped","offset":offset,"max_gap_ms":c.max_gap_ms}),
                )
                .await?;
        }
        if line_start {
            if ts.is_some() {
                source_count += 1
            } else {
                simulated_count += 1
            };
            previous = ts;
        }
        tokio::select! {_=stopped(&mut cx.shutdown)=>return Ok(()),_=tokio::time::sleep(delay)=>{}}
        let key = format!("{run}:{offset}");
        tokio::select! {_=stopped(&mut cx.shutdown)=>{eprintln!("replay stopped with {n} bytes not acknowledged");return Ok(())},r=cx.client.publish_retained(&c.stream,&key,payload,ts,BTreeMap::new())=>{r?;}}
        offset += n as u64;
        line_start = ends_line;
    }
    cx.client.request("report",json!({"state":"replay_complete","run":run,"bytes":offset,"source_timed_records":source_count,"simulated_records":simulated_count})).await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_explicit_and_json_times() {
        assert_eq!(timestamp(b"123 x", "unix_ms"), Some(123000000));
        assert_eq!(
            timestamp(b"{\"source_ts_ns\":42,\"data\":\"x\"}", "auto"),
            Some(42)
        );
        assert_eq!(
            timestamp(b"2026-09-08T12:00:00Z hello", "auto"),
            Some(1788868800000000000)
        );
        assert_eq!(timestamp(b"binary\xff", "auto"), None);
    }
}
