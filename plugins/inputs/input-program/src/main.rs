use anyhow::{bail, Context as _, Result};
use io_plugin_util::{bounded, stopped};
use log_plugin_sdk::Client;
use process_wrap::tokio::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    command: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: BTreeMap<String, String>,
    stdout_stream: String,
    stderr_stream: String,
    chunk_bytes: usize,
    shutdown_ms: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
            stdout_stream: "stdout".into(),
            stderr_stream: "stderr".into(),
            chunk_bytes: 4096,
            shutdown_ms: 2000,
        }
    }
}
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    if c.command.is_empty()
        || c.stdout_stream.is_empty()
        || c.stderr_stream.is_empty()
        || c.stdout_stream == c.stderr_stream
    {
        bail!("command required and stdout/stderr streams must be distinct")
    };
    bounded("chunk_bytes", c.chunk_bytes, 1, log_proto::MAX_PAYLOAD)?;
    bounded("shutdown_ms", c.shutdown_ms as usize, 10, 10000)?;
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut cx = io_plugin_util::connect(validate, &[]).await?;
    let client = cx.client.clone();
    let r = run(&mut cx).await;
    io_plugin_util::finish(&client, &r).await;
    r
}
async fn pump<R: AsyncRead + Unpin>(
    mut reader: R,
    client: &Client,
    stream: &str,
    run: uuid::Uuid,
    chunk: usize,
) -> Result<u64> {
    let mut buf = vec![0; chunk];
    let mut offset = 0u64;
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(offset);
        };
        let key = format!("{run}:{offset}");
        client
            .publish_retained(stream, &key, buf[..n].to_vec(), None, BTreeMap::new())
            .await?;
        offset += n as u64;
    }
}
async fn run(cx: &mut io_plugin_util::Context) -> Result<()> {
    let c = config(&cx.config.borrow())?;
    let mut command = CommandWrap::with_new(&c.command, |cmd| {
        cmd.args(&c.args)
            .envs(&c.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &c.cwd {
            cmd.current_dir(cwd);
        }
        for key in [
            "LOG_PRINT_CORE",
            "LOG_PRINT_PLUGIN",
            "LOG_PRINT_TOKEN",
            "LOG_PRINT_CONFIG",
        ] {
            cmd.env_remove(key);
        }
    });
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn {}", c.command))?;
    let run = uuid::Uuid::new_v4();
    let stdout = child.stdout().take().context("stdout pipe missing")?;
    let stderr = child.stderr().take().context("stderr pipe missing")?;
    let outcome:Result<()>=async {
        cx.client.request("report",json!({"state":"capturing","source_pid":child.id(),"run":run,"stdout_stream":c.stdout_stream,"stderr_stream":c.stderr_stream,"source_buffering":"controlled by source program"})).await?;
        let capture=async{tokio::try_join!(pump(stdout,&cx.client,&c.stdout_stream,run,c.chunk_bytes),pump(stderr,&cx.client,&c.stderr_stream,run,c.chunk_bytes))};
        tokio::select!{
            _=stopped(&mut cx.shutdown)=>{eprintln!("shutdown: source pipes may contain unacknowledged bytes; remaining count unknown");Ok(())},
            result=capture=>{
                let (out_bytes,err_bytes)=result?;
                let status=tokio::select!{_=stopped(&mut cx.shutdown)=>return Ok(()),status=child.wait()=>status?};
                cx.client.request("report",json!({"state":"source_exited","exit_code":status.code(),"stdout_bytes":out_bytes,"stderr_bytes":err_bytes})).await?;
                if !status.success(){bail!("source program exited with {status}")}Ok(())
            }
        }
    }.await;
    // Kill only the process group/job created above, including lingering descendants.
    if let Err(e) = child.start_kill() {
        if e.kind() != std::io::ErrorKind::InvalidInput && e.raw_os_error() != Some(3) {
            eprintln!("source group cleanup: {e}");
        }
    }
    match tokio::time::timeout(Duration::from_millis(c.shutdown_ms), child.wait()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => eprintln!("source reap failed: {e}"),
        Err(_) => eprintln!("source cleanup timed out; unconfirmed descendants reported"),
    }
    outcome
}
