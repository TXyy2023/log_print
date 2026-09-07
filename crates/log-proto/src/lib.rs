//! First formal protocol. All byte payloads are JSON arrays; framing is bounded JSONL.
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL: &str = "log-print/1";
pub const MAX_WIRE: usize = 1024 * 1024;
pub const MAX_PAYLOAD: usize = 64 * 1024;
pub const EVENT_QUEUE: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub protocol: String,
    pub plugin: String,
    pub token: String,
    #[serde(default)]
    pub events: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub op: String,
    #[serde(default)]
    pub args: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
}
impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Fault {}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Response {
        id: u64,
        result: Value,
        error: Option<Fault>,
    },
    Record {
        record: Record,
    },
    Gap {
        stream: String,
        epoch: String,
        from: u64,
        to: u64,
        reason: String,
    },
    Control {
        call_id: u64,
        method: String,
        args: Value,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub stream: String,
    pub epoch: String,
    pub seq: u64,
    pub key: String,
    pub payload: Vec<u8>,
    pub source_ts_ns: Option<u64>,
    pub observed_ts_ns: u64,
    #[serde(default)]
    pub upstream: BTreeMap<String, u64>,
    #[serde(default)]
    pub upstream_epochs: BTreeMap<String, String>,
    pub durability: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SaveOptions {
    pub enabled: Option<bool>,
    pub directory: Option<String>,
    pub file_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
}
impl SaveOptions {
    pub fn overlay(&self, other: &Self) -> Self {
        Self {
            enabled: other.enabled.or(self.enabled),
            directory: other.directory.clone().or_else(|| self.directory.clone()),
            file_bytes: other.file_bytes.or(self.file_bytes),
            total_bytes: other.total_bytes.or(self.total_bytes),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamSpec {
    pub id: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub save: SaveOptions,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginSpec {
    pub id: String,
    pub bin: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "yes")]
    pub autostart: bool,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub streams: Vec<StreamSpec>,
    #[serde(default)]
    pub save: SaveOptions,
    #[serde(default)]
    pub config: Value,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoreOptions {
    pub buffer_bytes: usize,
    pub buffer_records: usize,
    pub max_payload_bytes: usize,
    pub read_batch_records: usize,
    pub queue_records: usize,
    pub save: SaveOptions,
}
impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            buffer_bytes: 4 * 1024 * 1024,
            buffer_records: 4096,
            max_payload_bytes: MAX_PAYLOAD,
            read_batch_records: 64,
            queue_records: EVENT_QUEUE,
            save: SaveOptions {
                enabled: Some(false),
                directory: Some(".log-print/data".into()),
                file_bytes: Some(64 * 1024 * 1024),
                total_bytes: Some(1024 * 1024 * 1024),
            },
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub core: CoreOptions,
    #[serde(default)]
    pub plugins: Vec<PluginSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub config: Config,
    pub admin_token: String,
    pub plugin_tokens: BTreeMap<String, String>,
}
pub fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}
/// Incrementally rejects oversized frames, including peers that never send newline.
pub async fn read_json<R: AsyncBufRead + Unpin, T: for<'de> Deserialize<'de>>(
    r: &mut R,
) -> Result<Option<T>> {
    let mut frame = Vec::new();
    loop {
        let chunk = r.fill_buf().await?;
        if chunk.is_empty() {
            if frame.is_empty() {
                return Ok(None);
            }
            bail!("truncated_frame")
        }
        let n = chunk
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| n + 1)
            .unwrap_or(chunk.len());
        if frame.len() + n > MAX_WIRE {
            bail!("frame_too_large")
        }
        let done = chunk[n - 1] == b'\n';
        frame.extend_from_slice(&chunk[..n]);
        r.consume(n);
        if done {
            return Ok(Some(serde_json::from_slice(&frame)?));
        }
    }
}
pub async fn write_json<W: AsyncWrite + Unpin, T: Serialize>(w: &mut W, msg: &T) -> Result<()> {
    let mut bytes = serde_json::to_vec(msg)?;
    if bytes.len() + 1 > MAX_WIRE {
        bail!("frame_too_large")
    }
    bytes.push(b'\n');
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}
