//! Spawned programs and existing tmux panes each publish into one allocated
//! stream. Channels identify pipes; concurrent pipes have no shared source order.
use anyhow::{bail, Context as _, Result};
use log_plugin_sdk::{bounded, stopped, Client};
use process_wrap::tokio::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Mode {
    #[default]
    Spawn,
    Tmux,
}
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    mode: Mode,
    command: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    env: BTreeMap<String, String>,
    chunk_bytes: usize,
    shutdown_ms: u64,
    tmux_target: Option<String>,
    /// Exact tmux server socket path, passed as a single -S argument.
    tmux_socket: Option<PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Spawn,
            command: String::new(),
            args: vec![],
            cwd: None,
            env: BTreeMap::new(),
            chunk_bytes: 4096,
            shutdown_ms: 2000,
            tmux_target: None,
            tmux_socket: None,
        }
    }
}
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    match c.mode {
        Mode::Spawn => {
            if c.command.is_empty() {
                bail!("spawn mode requires command")
            }
            if c.tmux_target.is_some() || c.tmux_socket.is_some() {
                bail!("tmux_target and tmux_socket are only valid in tmux mode")
            }
        }
        Mode::Tmux => {
            if c.tmux_target
                .as_ref()
                .is_none_or(|target| target.is_empty())
            {
                bail!("tmux mode requires tmux_target (a pane or unambiguous target)")
            }
            if !c.command.is_empty() || !c.args.is_empty() || c.cwd.is_some() || !c.env.is_empty() {
                bail!("tmux mode does not start or modify a target program")
            }
        }
    }
    bounded("chunk_bytes", c.chunk_bytes, 1, log_proto::MAX_PAYLOAD)?;
    if !(10..=10000).contains(&c.shutdown_ms) {
        bail!("shutdown_ms must be 10..=10000")
    }
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
}

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(unix)]
    {
        let args: Vec<_> = std::env::args_os().collect();
        if args.get(1).is_some_and(|arg| arg == "--tmux-pipe") {
            if args.len() != 4 {
                bail!("internal tmux pipe requires socket and token")
            }
            return tmux::helper(
                PathBuf::from(&args[2]),
                args[3].to_string_lossy().into_owned(),
            )
            .await;
        }
    }
    let mut cx = log_plugin_sdk::connect(validate).await?;
    let client = cx.client.clone();
    let c = config(&cx.config)?;
    let result = match c.mode {
        Mode::Spawn => spawn(&mut cx, &c).await,
        Mode::Tmux => {
            #[cfg(unix)]
            {
                tmux::capture(&mut cx, &c).await
            }
            #[cfg(not(unix))]
            {
                Err(anyhow::anyhow!("tmux capture requires a Unix platform"))
            }
        }
    }
    .and_then(|_| cx.shutdown_result());
    log_plugin_sdk::finish(&client, &result).await;
    result
}

async fn pump<R: AsyncRead + Unpin>(
    mut reader: R,
    client: &Client,
    stream: &str,
    run: uuid::Uuid,
    channel: &str,
    chunk: usize,
) -> Result<u64> {
    let mut buf = vec![0; chunk];
    let mut offset = 0u64;
    let mut source_seq = 0u64;
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(offset);
        }
        source_seq += 1;
        let key = format!("{run}:{channel}:{offset}");
        client
            .publish_tagged(
                stream,
                &key,
                buf[..n].to_vec(),
                None,
                BTreeMap::new(),
                Some(channel.to_owned()),
                Some(source_seq),
            )
            .await?;
        offset += n as u64;
    }
}

async fn spawn(cx: &mut log_plugin_sdk::Context, c: &Config) -> Result<()> {
    let stream = cx
        .client
        .stream_id()
        .context("input stream was not allocated")?
        .to_owned();
    let mut command = CommandWrap::with_new(&c.command, |cmd| {
        cmd.args(&c.args)
            .envs(&c.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &c.cwd {
            cmd.current_dir(cwd);
        }
        // Do not disclose the plugin's Core capability to the target program.
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("LOG_PRINT_") {
                cmd.env_remove(key);
            }
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
    let outcome: Result<()> = async {
        cx.client.request("report", json!({"state":"capturing","source_pid":child.id(),"run":run,"stream":stream,"channels":["stdout","stderr"],"source_order":"independent per channel; merged by reception","source_buffering":"controlled by source program"})).await?;
        let capture = async { tokio::try_join!(pump(stdout,&cx.client,&stream,run,"stdout",c.chunk_bytes),pump(stderr,&cx.client,&stream,run,"stderr",c.chunk_bytes)) };
        tokio::select! {
            _=stopped(&mut cx.shutdown)=>{ eprintln!("shutdown: source pipes may contain unpublished bytes; remaining count unknown"); Ok(()) },
            result=capture=>{
                let (out_bytes,err_bytes)=result?;
                let status=tokio::select! { _=stopped(&mut cx.shutdown)=>return Ok(()), status=child.wait()=>status? };
                cx.client.request("report", json!({"state":"source_exited","exit_code":status.code(),"stdout_bytes":out_bytes,"stderr_bytes":err_bytes,"downstream_complete":false})).await?;
                if !status.success() { bail!("source program exited with {status}") }
                Ok(())
            }
        }
    }.await;
    // Only the process group/job that we created is terminated. Inherited pipe
    // holders are included, so manual stop does not leave descendants running.
    if let Err(e) = child.start_kill() {
        if e.kind() != std::io::ErrorKind::InvalidInput && e.raw_os_error() != Some(3) {
            eprintln!("source group cleanup: {e}");
        }
    }
    match tokio::time::timeout(Duration::from_millis(c.shutdown_ms), child.wait()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => eprintln!("source reap failed: {e}"),
        Err(_) => eprintln!("source cleanup timed out; descendant exit not confirmed"),
    }
    outcome
}

#[cfg(unix)]
mod tmux {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;
    use tokio::{
        io::AsyncWriteExt,
        net::{UnixListener, UnixStream},
        process::Command,
    };

    struct PrivateSocket {
        dir: PathBuf,
        path: PathBuf,
    }
    impl PrivateSocket {
        fn new() -> Result<Self> {
            // A short path also fits macOS's sockaddr_un path limit.
            let dir = PathBuf::from("/tmp").join(format!("log-print-{}", uuid::Uuid::new_v4()));
            std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
            let path = dir.join("pipe.sock");
            Ok(Self { dir, path })
        }
    }
    impl Drop for PrivateSocket {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_dir(&self.dir);
        }
    }
    fn shell_quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
    async fn command(c: &Config, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new("tmux");
        if let Some(socket) = &c.tmux_socket {
            cmd.arg("-S").arg(socket);
        }
        let output = tokio::time::timeout(
            Duration::from_secs(5),
            cmd.args(args).kill_on_drop(true).output(),
        )
        .await
        .context("tmux command timed out")??;
        if !output.status.success() {
            bail!(
                "tmux failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    /// Called by tmux pipe-pane. The socket's reverse direction exists only as
    /// a lifetime signal: collector closure ends this helper even on idle panes.
    /// No stop path calls pipe-pane, so a later third-party pipe is never closed.
    pub(super) async fn helper(path: PathBuf, token: String) -> Result<()> {
        let mut socket = UnixStream::connect(path).await?;
        socket.write_all(format!("{token}\n").as_bytes()).await?;
        let (mut lifetime, mut writer) = socket.into_split();
        let mut stdin = tokio::io::stdin();
        let mut byte = [0u8; 1];
        tokio::select! {
            sent=tokio::io::copy(&mut stdin,&mut writer)=>{ sent?; },
            end=lifetime.read(&mut byte)=>{ end?; }
        }
        // Tokio stdin uses a blocking thread that may otherwise prevent runtime
        // shutdown while an idle tmux pane holds its pipe open.
        std::process::exit(0)
    }

    pub(super) async fn capture(cx: &mut log_plugin_sdk::Context, c: &Config) -> Result<()> {
        let stream = cx
            .client
            .stream_id()
            .context("input stream was not allocated")?
            .to_owned();
        let target = c.tmux_target.as_deref().context("tmux_target required")?;
        let info = command(
            c,
            &[
                "display-message",
                "-p",
                "-t",
                target,
                "#{pane_id}\t#{pane_pipe}",
            ],
        )
        .await?;
        let (pane, piped) = info
            .split_once('\t')
            .context("unexpected tmux pane response")?;
        if piped != "0" {
            bail!("tmux pane {pane} already has pipe-pane; refusing to replace it")
        }
        let private = PrivateSocket::new()?;
        let listener = UnixListener::bind(&private.path)?;
        let token = uuid::Uuid::new_v4().to_string();
        let exe = std::env::current_exe()?;
        let pipe_command = format!(
            "exec {} --tmux-pipe {} {}",
            shell_quote(exe.to_str().context("executable path is not UTF-8")?),
            shell_quote(private.path.to_str().context("socket path is not UTF-8")?),
            shell_quote(&token)
        );
        // Check and install within tmux's synchronous command queue. Do not use
        // pipe-pane -o: that flag toggles an existing pipe OFF on a race.
        // -I lets tmux notice helper EOF on an idle pane immediately. The helper
        // never writes stdout, so it cannot inject terminal input.
        let install = format!(
            "pipe-pane -I -O -t {} {}",
            shell_quote(pane),
            shell_quote(&pipe_command)
        );
        let installed = command(
            c,
            &[
                "if-shell",
                "-F",
                "-t",
                pane,
                "#{pane_pipe}",
                "display-message -p log-print-pipe-busy",
                &install,
            ],
        )
        .await?;
        if installed == "log-print-pipe-busy" {
            bail!("tmux pane {pane} already has pipe-pane; refusing concurrent replacement")
        }
        let handshake = async {
            let (mut connection, _) = listener.accept().await?;
            let expected = format!("{token}\n");
            let mut received = vec![0; expected.len()];
            connection.read_exact(&mut received).await?;
            if received != expected.as_bytes() {
                bail!("tmux helper handshake mismatch")
            }
            Ok::<_, anyhow::Error>(connection)
        };
        let connection = tokio::select! {
            _=stopped(&mut cx.shutdown)=>return Ok(()),
            result=tokio::time::timeout(Duration::from_secs(5),handshake)=>result.context("tmux pipe did not connect; it may have been claimed concurrently")??
        };
        drop(listener);
        cx.client.request("report", json!({"state":"capturing","mode":"tmux","pane":pane,"stream":stream,"channel":"terminal","history_imported":false,"starts_at":"pipe attachment","stopping":"disconnects only this collector; target program keeps running"})).await?;
        let run = uuid::Uuid::new_v4();
        tokio::select! {
            _=stopped(&mut cx.shutdown)=>Ok(()),
            result=pump(connection,&cx.client,&stream,run,"terminal",c.chunk_bytes)=>{
                let bytes=result?;
                cx.client.request("report", json!({"state":"source_eof","mode":"tmux","bytes_sent":bytes,"downstream_complete":false})).await?;
                Ok(())
            }
        }
    }

    #[test]
    fn quotes_shell_metacharacters_as_data() {
        assert_eq!(shell_quote("a'b $x;`id`"), "'a'\\''b $x;`id`'");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_validation_prevents_unintended_target_changes() {
        assert!(config(&json!({"command":"echo"})).is_ok());
        assert!(config(&json!({"mode":"tmux","tmux_target":"%1"})).is_ok());
        for bad in [
            json!({}),
            json!({"mode":"tmux"}),
            json!({"mode":"tmux","tmux_target":"%1","command":"echo"}),
            json!({"command":"echo","tmux_target":"%1"}),
            json!({"command":"echo","stdout_stream":"old"}),
            json!({"command":"echo","shutdown_ms":0}),
        ] {
            assert!(config(&bad).is_err(), "{bad}");
        }
    }
}
