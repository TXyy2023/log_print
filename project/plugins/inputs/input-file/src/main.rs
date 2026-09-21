//! File bytes are published as one Core-assigned stream. Static completion means
//! source EOF/publication only; it says nothing about downstream consumption.
use anyhow::{bail, Context as _, Result};
use log_plugin_sdk::{bounded, stopped};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::SeekFrom, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Follow,
    Static,
}
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    path: PathBuf,
    mode: Mode,
    from_start: bool,
    chunk_bytes: usize,
    poll_ms: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            mode: Mode::Follow,
            from_start: false,
            chunk_bytes: 4096,
            poll_ms: 50,
        }
    }
}
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    if c.path.as_os_str().is_empty() {
        bail!("path required")
    }
    bounded("chunk_bytes", c.chunk_bytes, 1, log_proto::MAX_PAYLOAD)?;
    if !(1..=60000).contains(&c.poll_ms) {
        bail!("poll_ms must be 1..=60000")
    }
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
}

fn open_regular(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Opening a FIFO with no writer must not hang before type validation.
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "input-file requires a regular file",
        ));
    }
    Ok(file)
}

struct Prepared {
    file: std::fs::File,
    identity: same_file::Handle,
    offset: u64,
    anchor: Vec<u8>,
}
impl Prepared {
    fn open(c: &Config) -> Result<Self> {
        use std::io::{Read, Seek};
        let mut file =
            open_regular(&c.path).with_context(|| format!("open {}", c.path.display()))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            bail!("input-file requires a regular file")
        }
        let identity = same_file::Handle::from_file(file.try_clone()?)?;
        let offset = if c.mode == Mode::Static || c.from_start {
            0
        } else {
            metadata.len()
        };
        let mut anchor = vec![0; offset.min(64) as usize];
        file.seek(SeekFrom::Start(offset - anchor.len() as u64))?;
        file.read_exact(&mut anchor)?;
        Ok(Self {
            file,
            identity,
            offset,
            anchor,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Fix follow's initial position before registration. Bytes appended while
    // connecting must not accidentally become skipped startup history.
    let initial = config(&serde_json::from_str(
        &std::env::var("LOG_PRINT_CONFIG").unwrap_or_else(|_| "{}".into()),
    )?)?;
    let prepared = Prepared::open(&initial)?;
    let mut cx = log_plugin_sdk::connect(validate).await?;
    let client = cx.client.clone();
    let result = run(&mut cx, prepared)
        .await
        .and_then(|_| cx.shutdown_result());
    log_plugin_sdk::finish(&client, &result).await;
    result
}
async fn run(cx: &mut log_plugin_sdk::Context, prepared: Prepared) -> Result<()> {
    let c = config(&cx.config)?;
    let stream = cx
        .client
        .stream_id()
        .context("input stream was not allocated")?
        .to_owned();
    let Prepared {
        file,
        mut identity,
        mut offset,
        mut anchor,
    } = prepared;
    let mut file = tokio::fs::File::from_std(file);
    let run = uuid::Uuid::new_v4();
    let mut segment = 0u64;
    let mut source_seq = 0u64;
    let mut total_bytes = 0u64;
    let mut absent = false;
    cx.client.request("report", json!({"state":if c.mode == Mode::Static {"reading"} else {"following"},"path":c.path,"stream":stream,"segment":segment,"offset":offset,"from_start":c.mode == Mode::Static || c.from_start,"run":run})).await?;
    let mut buf = vec![0; c.chunk_bytes];
    loop {
        if *cx.shutdown.borrow() {
            return Ok(());
        }
        if c.mode == Mode::Follow {
            let candidate = match open_regular(&c.path) {
                Ok(h) => h,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if !absent {
                        cx.client.request("report", json!({"state":"path_missing","segment":segment,"offset":offset,"possible_missing_bytes":"unknown"})).await?;
                        absent = true;
                    }
                    tokio::select! { _=stopped(&mut cx.shutdown)=>return Ok(()), _=tokio::time::sleep(Duration::from_millis(c.poll_ms))=>{} }
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let candidate_identity = same_file::Handle::from_file(candidate.try_clone()?)?;
            let replaced = candidate_identity != identity;
            let len = file.metadata().await?.len();
            let mut rewritten = false;
            if !replaced && len >= offset && !anchor.is_empty() {
                file.seek(SeekFrom::Start(offset - anchor.len() as u64))
                    .await?;
                let mut check = vec![0; anchor.len()];
                rewritten = match file.read_exact(&mut check).await {
                    Ok(_) => check != anchor,
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => true,
                    Err(e) => return Err(e.into()),
                };
                file.seek(SeekFrom::Start(offset)).await?;
            }
            if replaced || len < offset || rewritten {
                let reason = if replaced {
                    "replaced"
                } else if rewritten {
                    "rewritten_or_truncated"
                } else {
                    "truncated"
                };
                let old_offset = offset;
                segment += 1;
                offset = 0;
                anchor.clear();
                if replaced {
                    identity = candidate_identity;
                    file = tokio::fs::File::from_std(candidate);
                }
                file.seek(SeekFrom::Start(0)).await?;
                cx.client.request("report", json!({"state":"new_segment","reason":reason,"segment":segment,"previous_offset":old_offset,"offset":0,"possible_missing_bytes":"unknown"})).await?;
            } else if absent {
                cx.client
                    .request(
                        "report",
                        json!({"state":"path_restored","segment":segment,"offset":offset}),
                    )
                    .await?;
            }
            absent = false;
        }
        let n = tokio::select! { _=stopped(&mut cx.shutdown)=>return Ok(()), r=file.read(&mut buf)=>r? };
        if n > 0 {
            source_seq += 1;
            let key = format!("{run}:{segment}:{offset}");
            tokio::select! {
                _=stopped(&mut cx.shutdown)=>{ eprintln!("shutdown: input-file has {n} pending bytes; Core acceptance unknown"); return Ok(()) },
                r=cx.client.publish_tagged(&stream,&key,buf[..n].to_vec(),None,BTreeMap::new(),None,Some(source_seq))=>{r?;}
            }
            offset += n as u64;
            total_bytes += n as u64;
            anchor.extend_from_slice(&buf[..n]);
            if anchor.len() > 64 {
                anchor.drain(..anchor.len() - 64);
            }
        } else if c.mode == Mode::Static {
            cx.client.request("report", json!({"state":"source_eof","stream":stream,"bytes_sent":total_bytes,"chunks_sent":source_seq,"downstream_complete":false,"delivery":"transport-dependent; rolling buffer may overwrite unread records"})).await?;
            return Ok(());
        } else {
            tokio::select! { _=stopped(&mut cx.shutdown)=>return Ok(()), _=tokio::time::sleep(Duration::from_millis(c.poll_ms))=>{} }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_and_bounds() {
        assert_eq!(config(&json!({"path":"x"})).unwrap().mode, Mode::Follow);
        assert_eq!(
            config(&json!({"path":"x","mode":"static"})).unwrap().mode,
            Mode::Static
        );
        for bad in [
            json!({}),
            json!({"path":"x","chunk_bytes":0}),
            json!({"path":"x","poll_ms":0}),
            json!({"path":"x","mode":"replay"}),
            json!({"path":"x","stream":"old"}),
        ] {
            assert!(config(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn static_starts_at_zero_and_follow_uses_open_time_end() {
        let path = std::env::temp_dir().join(format!("log-print-file-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, b"history").unwrap();
        let tail = Prepared::open(&config(&json!({"path":path})).unwrap()).unwrap();
        let full = Prepared::open(&config(&json!({"path":path,"mode":"static"})).unwrap()).unwrap();
        assert_eq!(tail.offset, 7);
        assert_eq!(tail.anchor, b"history");
        assert_eq!(full.offset, 0);
        std::fs::remove_file(path).unwrap();
    }
}
