use anyhow::{bail, Context as _, Result};
use io_plugin_util::{bounded, stopped};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::SeekFrom, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    path: PathBuf,
    stream: String,
    from_start: bool,
    chunk_bytes: usize,
    poll_ms: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            stream: "file".into(),
            from_start: false,
            chunk_bytes: 4096,
            poll_ms: 50,
        }
    }
}
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    if c.path.as_os_str().is_empty() || c.stream.is_empty() {
        bail!("path and stream required")
    };
    bounded("chunk_bytes", c.chunk_bytes, 1, log_proto::MAX_PAYLOAD)?;
    bounded("poll_ms", c.poll_ms as usize, 1, 60000)?;
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
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
            std::fs::File::open(&c.path).with_context(|| format!("open {}", c.path.display()))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            bail!("input-file requires a regular file")
        }
        let identity = same_file::Handle::from_file(file.try_clone()?)?;
        let offset = if c.from_start { 0 } else { metadata.len() };
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
    // Establish tail's starting position before Core can report us connected.
    // Bytes appended after registration begins must not become skipped history.
    let initial = config(&serde_json::from_str(
        &std::env::var("LOG_PRINT_CONFIG").unwrap_or_else(|_| "{}".into()),
    )?)?;
    let prepared = Prepared::open(&initial)?;
    let mut cx = io_plugin_util::connect(validate, &["poll_ms"]).await?;
    let client = cx.client.clone();
    let result = run(&mut cx, prepared).await;
    io_plugin_util::finish(&client, &result).await;
    result
}
async fn run(cx: &mut io_plugin_util::Context, prepared: Prepared) -> Result<()> {
    let c = config(&cx.config.borrow())?;
    let Prepared {
        file,
        mut identity,
        mut offset,
        mut anchor,
    } = prepared;
    let mut file = tokio::fs::File::from_std(file);
    let run = uuid::Uuid::new_v4();
    let mut segment = 0u64;
    let mut absent = false;
    cx.client.request("report",json!({"state":"following","path":c.path,"segment":segment,"offset":offset,"from_start":c.from_start,"run":run})).await?;
    let mut buf = vec![0; c.chunk_bytes];
    loop {
        if *cx.shutdown.borrow() {
            return Ok(());
        }
        let path_handle = match same_file::Handle::from_path(&c.path) {
            Ok(h) => h,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !absent {
                    cx.client.request("report",json!({"state":"path_missing","segment":segment,"offset":offset,"possible_missing_bytes":"unknown"})).await?;
                    absent = true;
                }
                let poll = config(&cx.config.borrow())?.poll_ms;
                tokio::select! {_=stopped(&mut cx.shutdown)=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(poll))=>{}};
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let replaced = path_handle != identity;
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
                let new = std::fs::File::open(&c.path)?;
                if !new.metadata()?.is_file() {
                    bail!("replacement is not a regular file")
                };
                identity = same_file::Handle::from_file(new.try_clone()?)?;
                file = tokio::fs::File::from_std(new);
            }
            file.seek(SeekFrom::Start(0)).await?;
            cx.client.request("report",json!({"state":"new_segment","reason":reason,"segment":segment,"previous_offset":old_offset,"offset":0,"possible_missing_bytes":"unknown"})).await?;
        } else if absent {
            cx.client
                .request(
                    "report",
                    json!({"state":"path_restored","segment":segment,"offset":offset}),
                )
                .await?;
        }
        absent = false;
        let n =
            tokio::select! {_=stopped(&mut cx.shutdown)=>return Ok(()),r=file.read(&mut buf)=>r?};
        if n > 0 {
            let key = format!("{run}:{segment}:{offset}");
            tokio::select! {_=stopped(&mut cx.shutdown)=>{eprintln!("shutdown: pending input-file bytes not acknowledged: {n}");return Ok(())},r=cx.client.publish_retained(&c.stream,&key,buf[..n].to_vec(),None,BTreeMap::new())=>{r?;}}
            offset += n as u64;
            anchor.extend_from_slice(&buf[..n]);
            if anchor.len() > 64 {
                anchor.drain(..anchor.len() - 64);
            }
        } else {
            let poll = config(&cx.config.borrow())?.poll_ms;
            tokio::select! {_=stopped(&mut cx.shutdown)=>return Ok(()),_=tokio::time::sleep(Duration::from_millis(poll))=>{}};
        }
    }
}
