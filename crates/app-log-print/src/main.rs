//! Local supervisor and user CLI. Core and plugins are direct children.
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use log_proto::{
    read_json, write_json, Config, Fault, Hello, PluginSpec, Request, RuntimeConfig, ServerMessage,
    PROTOCOL,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "log-print",
    version,
    about = "Local multi-stream logs, independent plugins and live views"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".log-print/state.json")]
    state: PathBuf,
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Run the supervisor in the foreground; Ctrl-C shuts down its children.
    Run {
        #[arg(long)]
        config: PathBuf,
    },
    /// Start a background supervisor and wait for Core readiness.
    Start {
        #[arg(long)]
        config: PathBuf,
    },
    Status,
    Streams,
    Read {
        stream: String,
        #[arg(long, default_value_t = 1)]
        from: u64,
        #[arg(long, default_value_t = 64)]
        limit: usize,
        #[arg(long)]
        epoch: Option<String>,
        /// Wait for a nonempty page or gap, up to this many milliseconds (0 = snapshot).
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=60_000))]
        wait_ms: u64,
        /// Write only record payload bytes; refuse pages that report gaps.
        #[arg(long)]
        raw: bool,
    },
    Config {
        #[arg(long)]
        plugin: Option<String>,
        #[command(subcommand)]
        command: Option<ConfigAction>,
    },
    Plugin {
        #[command(subcommand)]
        command: PluginAction,
    },
    Session {
        #[command(subcommand)]
        command: SessionAction,
    },
    /// Call any Core RPC with a JSON argument object.
    Call {
        op: String,
        #[arg(long, default_value = "{}")]
        json: String,
    },
    /// Stop this instance and wait for its state file to be removed.
    Stop,
}
#[derive(Subcommand)]
enum ConfigAction {
    /// Ask a running plugin to apply supported runtime configuration fields.
    Set {
        plugin: String,
        #[arg(long)]
        json: String,
    },
}
#[derive(Subcommand)]
enum PluginAction {
    Start {
        id: String,
    },
    Stop {
        id: String,
    },
    Restart {
        id: String,
    },
    Call {
        id: String,
        method: String,
        #[arg(long, default_value = "{}")]
        json: String,
    },
}
#[derive(Subcommand)]
enum SessionAction {
    List {
        plugin: String,
    },
    Create {
        plugin: String,
        #[arg(long)]
        json: String,
    },
    Get {
        plugin: String,
        id: String,
    },
    Select {
        plugin: String,
        id: String,
    },
    Set {
        plugin: String,
        id: String,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        json: String,
    },
    Export {
        plugin: String,
        id: String,
        #[arg(long)]
        path: PathBuf,
        #[arg(long, default_value = "png", value_parser = ["png", "svg"])]
        format: String,
        #[arg(long)]
        revision: Option<u64>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long)]
        overwrite: bool,
    },
}

#[derive(Clone, Serialize, Deserialize)]
struct State {
    protocol: String,
    address: String,
    token: String,
    pid: u32,
    core_pid: u32,
    core_address: String,
    config: String,
    runtime_directory: String,
}
#[derive(Deserialize)]
struct Ready {
    address: String,
    pid: u32,
}

fn absolute(path: &Path) -> Result<PathBuf> {
    Ok(if path.is_absolute() {
        path.into()
    } else {
        std::env::current_dir()?.join(path)
    })
}
fn private_directory(path: &Path) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .with_context(|| format!("create {}", path.display()))?;
    restrict_access(path, true)?;
    Ok(())
}
fn private_file(path: &Path, exclusive: bool, append: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create(!exclusive)
        .create_new(exclusive)
        .append(append);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    if exclusive {
        if let Err(error) = restrict_access(path, false) {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(error);
        }
    }
    Ok(file)
}
#[cfg(not(windows))]
fn restrict_access(_path: &Path, _directory: bool) -> Result<()> {
    Ok(())
}
#[cfg(windows)]
fn restrict_access(path: &Path, directory: bool) -> Result<()> {
    let user = std::env::var("USERNAME").context("USERNAME required for private runtime ACL")?;
    let account = match std::env::var("USERDOMAIN") {
        Ok(domain) => format!("{domain}\\{user}"),
        Err(_) => user,
    };
    let rights = if directory { "(OI)(CI)F" } else { "F" };
    let result = std::process::Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{account}:{rights}"))
        .output()
        .context("restrict runtime ACL")?;
    if !result.status.success() {
        bail!(
            "cannot restrict runtime ACL for {}: {}",
            path.display(),
            String::from_utf8_lossy(&result.stderr)
        )
    }
    Ok(())
}
fn save_json(file: &mut File, value: &impl Serialize) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    let bytes = serde_json::to_vec_pretty(value)?;
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}
fn log_path(state: &Path, extension: &str) -> PathBuf {
    let mut name = state.as_os_str().to_os_string();
    name.push(extension);
    PathBuf::from(name)
}
fn parse_args(text: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(text).context("invalid --json")?;
    if !value.is_object() {
        bail!("--json must be a JSON object")
    }
    Ok(value)
}
fn resolve_binary(name: &str, config_dir: &Path) -> Result<PathBuf> {
    let path = Path::new(name);
    if path.is_absolute() {
        return Ok(path.into());
    }
    if path.components().count() > 1 {
        return Ok(config_dir.join(path));
    }
    let sibling = std::env::current_exe()?
        .parent()
        .context("executable directory")?
        .join(path);
    #[cfg(windows)]
    let sibling = {
        let mut p = sibling;
        if p.extension().is_none() {
            p.set_extension("exe");
        }
        p
    };
    if sibling.exists() {
        Ok(sibling)
    } else {
        Ok(path.into())
    }
}

/// One authenticated request. There is no shared event queue that can block CLI replies.
async fn rpc(address: &str, identity: &str, token: &str, op: &str, args: Value) -> Result<Value> {
    tokio::time::timeout(
        Duration::from_secs(if identity == "__manager__" { 30 } else { 15 }),
        async {
            let address: std::net::SocketAddr = address.parse().context("invalid local address")?;
            if !address.ip().is_loopback() {
                bail!("refusing non-loopback IPC address")
            }
            let stream = TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            let (read, mut write) = stream.into_split();
            let mut read = BufReader::new(read);
            write_json(
                &mut write,
                &Hello {
                    protocol: PROTOCOL.into(),
                    plugin: identity.into(),
                    token: token.into(),
                    events: false,
                },
            )
            .await?;
            match read_json::<_, ServerMessage>(&mut read)
                .await?
                .context("EOF during handshake")?
            {
                ServerMessage::Response {
                    id: 0,
                    error: Some(error),
                    ..
                } => return Err(error.into()),
                ServerMessage::Response {
                    id: 0, error: None, ..
                } => (),
                _ => bail!("invalid handshake reply"),
            }
            write_json(
                &mut write,
                &Request {
                    id: 1,
                    op: op.into(),
                    args,
                },
            )
            .await?;
            loop {
                match read_json::<_, ServerMessage>(&mut read)
                    .await?
                    .context("Core disconnected; operation result may be unknown")?
                {
                    ServerMessage::Response {
                        id: 1,
                        result,
                        error: None,
                    } => return Ok(result),
                    ServerMessage::Response {
                        id: 1,
                        error: Some(error),
                        ..
                    } => return Err(error.into()),
                    _ => (),
                }
            }
        },
    )
    .await
    .context("RPC timed out; operation result may be unknown; not retried")?
}

struct Files {
    state: PathBuf,
    directory: PathBuf,
    file: Option<File>,
}
impl Drop for Files {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.state);
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
struct Managed {
    spec: PluginSpec,
    child: Option<Child>,
    last_exit: Option<String>,
    last_success: Option<bool>,
}
struct Supervisor {
    state: State,
    config: Config,
    config_dir: PathBuf,
    tokens: BTreeMap<String, String>,
    plugins: BTreeMap<String, Managed>,
    core: Child,
    core_stdin: Option<ChildStdin>,
    stopping: bool,
}
impl Supervisor {
    async fn core_rpc(&self, op: &str, args: Value) -> Result<Value> {
        rpc(
            &self.state.core_address,
            "__admin__",
            &self.state.token,
            op,
            args,
        )
        .await
    }
    fn refresh(&mut self) -> Result<()> {
        for (id, plugin) in &mut self.plugins {
            if let Some(child) = plugin.child.as_mut() {
                if let Some(status) = child.try_wait()? {
                    plugin.last_exit = Some(status.to_string());
                    plugin.last_success = Some(status.success());
                    plugin.child = None;
                    eprintln!(
                        "[supervisor] plugin {id} exited: {status}; manual restart available"
                    );
                }
            }
        }
        Ok(())
    }
    fn process_status(&self) -> Vec<Value> {
        self.plugins.iter().map(|(id, p)| json!({"id":id,"pid":p.child.as_ref().and_then(Child::id),"state":if p.child.is_some(){"running"}else{"stopped"},"last_exit":p.last_exit,"autostart":p.spec.autostart})).collect()
    }
    fn start_plugin(&mut self, id: &str) -> Result<Value> {
        self.refresh()?;
        let managed = self
            .plugins
            .get_mut(id)
            .with_context(|| format!("unknown plugin: {id}"))?;
        if managed.child.is_some() {
            bail!("plugin {id} is already running")
        }
        let binary = resolve_binary(&managed.spec.bin, &self.config_dir)?;
        let child = Command::new(binary)
            .args(&managed.spec.args)
            .env("LOG_PRINT_CORE", &self.state.core_address)
            .env("LOG_PRINT_PLUGIN", id)
            .env("LOG_PRINT_TOKEN", &self.tokens[id])
            .env(
                "LOG_PRINT_CONFIG",
                serde_json::to_string(&managed.spec.config)?,
            )
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("start plugin {id}"))?;
        let pid = child.id();
        managed.child = Some(child);
        managed.last_exit = None;
        managed.last_success = None;
        eprintln!("[supervisor] spawned plugin {id} pid={pid:?}");
        Ok(json!({"id":id,"pid":pid,"state":"started"}))
    }
    async fn wait_plugins(&mut self, ids: &[String]) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            self.refresh()?;
            let status = self.core_rpc("status", json!({})).await?;
            let plugins = status["plugins"]
                .as_array()
                .context("Core status missing plugins")?;
            let mut waiting = Vec::new();
            for id in ids {
                let plugin = plugins
                    .iter()
                    .find(|p| p["id"].as_str() == Some(id))
                    .context("Core status missing configured plugin")?;
                let report = plugin["report"]["state"].as_str();
                if report == Some("failed") {
                    bail!("plugin {id} initialization failed: {}", plugin["report"])
                }
                if plugin["connected"] == true {
                    continue;
                }
                if self.plugins[id].child.is_none() {
                    if self.plugins[id].last_success == Some(true)
                        && matches!(report, Some("stopped" | "completed"))
                    {
                        continue;
                    }
                    bail!(
                        "plugin {id} exited before connecting: {:?}",
                        self.plugins[id].last_exit
                    )
                }
                waiting.push(id.as_str());
            }
            if waiting.is_empty() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("plugin readiness timed out: {}", waiting.join(", "))
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
    async fn stop_plugin(&mut self, id: &str, core_alive: bool) -> Result<Value> {
        self.refresh()?;
        let managed = self
            .plugins
            .get_mut(id)
            .with_context(|| format!("unknown plugin: {id}"))?;
        let Some(child) = managed.child.take() else {
            return Ok(json!({"id":id,"state":"stopped","already_stopped":true}));
        };
        let result = finish_plugin(
            id.to_owned(),
            child,
            self.state.core_address.clone(),
            self.state.token.clone(),
            core_alive,
        )
        .await;
        managed.last_exit = Some(result["exit"].as_str().unwrap_or("unknown").to_owned());
        managed.last_success = result["success"].as_bool();
        // Wait for the old RPC connection to leave Core before reusing this identity.
        if core_alive {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while tokio::time::Instant::now() < deadline {
                let status = match self.core_rpc("status", json!({})).await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                let connected = status["plugins"]
                    .as_array()
                    .is_some_and(|ps| ps.iter().any(|p| p["id"] == id && p["connected"] == true));
                if !connected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        Ok(result)
    }
    async fn handle(&mut self, op: &str, args: Value) -> Result<Value> {
        self.refresh()?;
        let plugin_id =
            || -> Result<String> { Ok(args["id"].as_str().context("id is required")?.to_owned()) };
        match op {
            "status" => {
                let mut status = self.core_rpc("status", json!({})).await?;
                status["supervisor"] = json!({"pid":self.state.pid,"core_pid":self.state.core_pid,"config":self.state.config});
                status["plugin_processes"] = json!(self.process_status());
                Ok(status)
            }
            "streams" => Ok(self.core_rpc("status", json!({})).await?["streams"].clone()),
            "config" => {
                if let Some(id) = args["plugin"].as_str() {
                    let p = self.plugins.get(id).context("unknown plugin")?;
                    let mut answer = json!({"id":id,"startup":p.spec,"source":self.state.config,"persisted":false});
                    match self
                        .core_rpc(
                            "control",
                            json!({"target":id,"method":"config.get","args":{}}),
                        )
                        .await
                    {
                        Ok(runtime) => answer["runtime"] = runtime,
                        Err(error) => answer["runtime_error"] = json!(error.to_string()),
                    }
                    Ok(answer)
                } else {
                    Ok(
                        json!({"config":self.config,"source":self.state.config,"runtime_overrides":"not persisted; query status for live plugin reports"}),
                    )
                }
            }
            "plugin.start" => {
                let id = plugin_id()?;
                let result = self.start_plugin(&id)?;
                if let Err(error) = self.wait_plugins(std::slice::from_ref(&id)).await {
                    let _ = self.stop_plugin(&id, true).await;
                    return Err(error);
                }
                Ok(result)
            }
            "plugin.stop" => self.stop_plugin(&plugin_id()?, true).await,
            "plugin.restart" => {
                let id = plugin_id()?;
                let stopped = self.stop_plugin(&id, true).await?;
                let started = self.start_plugin(&id)?;
                if let Err(error) = self.wait_plugins(std::slice::from_ref(&id)).await {
                    let _ = self.stop_plugin(&id, true).await;
                    return Err(error);
                }
                Ok(json!({"stopped":stopped,"started":started}))
            }
            "stop" => {
                self.stopping = true;
                Ok(json!({"stopping":true,"pid":self.state.pid}))
            }
            "core.call" => {
                let operation = args["op"].as_str().context("op is required")?;
                self.core_rpc(operation, args["args"].clone()).await
            }
            _ => bail!("unknown management operation: {op}"),
        }
    }
    async fn cleanup(&mut self, core_alive: bool) {
        let mut pending = JoinSet::new();
        for (id, managed) in &mut self.plugins {
            if let Some(child) = managed.child.take() {
                pending.spawn(finish_plugin(
                    id.clone(),
                    child,
                    self.state.core_address.clone(),
                    self.state.token.clone(),
                    core_alive,
                ));
            }
        }
        while let Some(result) = pending.join_next().await {
            match result {
                Ok(value) => eprintln!("[supervisor] shutdown {value}"),
                Err(e) => eprintln!("[supervisor] cleanup error: {e}"),
            }
        }
        drop(self.core_stdin.take());
        match tokio::time::timeout(Duration::from_secs(4), self.core.wait()).await {
            Ok(Ok(status)) => eprintln!("[supervisor] Core reaped: {status}"),
            other => {
                eprintln!("[supervisor] Core graceful shutdown incomplete: {other:?}; terminating owned child");
                let _ = self.core.start_kill();
                let _ = self.core.wait().await;
            }
        }
    }
}
async fn finish_plugin(
    id: String,
    mut child: Child,
    address: String,
    token: String,
    core_alive: bool,
) -> Value {
    let mut control_error = None;
    let mut forced = !core_alive;
    if core_alive {
        match tokio::time::timeout(
            Duration::from_secs(3),
            rpc(
                &address,
                "__admin__",
                &token,
                "control",
                json!({"target":id,"method":"shutdown","args":{}}),
            ),
        )
        .await
        {
            Ok(Ok(_)) => (),
            Ok(Err(e)) => control_error = Some(e.to_string()),
            Err(_) => {
                control_error =
                    Some("shutdown control timed out; unfinished data may remain".to_owned())
            }
        }
        match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
            Ok(Ok(status)) => {
                return json!({"id":id,"state":"stopped","exit":status.to_string(),"success":status.success(),"forced":false,"control_error":control_error})
            }
            _ => forced = true,
        }
    }
    let _ = child.start_kill();
    let exit = match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
        Ok(Ok(status)) => status.to_string(),
        other => format!("reap incomplete: {other:?}"),
    };
    json!({"id":id,"state":"stopped","exit":exit,"forced":forced,"control_error":control_error,"unfinished_data":if forced {"unknown; forced termination"} else {"reported by plugin"}})
}

type ManagementCall = (Request, oneshot::Sender<Result<Value>>);
async fn management_connection(
    stream: TcpStream,
    token: String,
    send: mpsc::Sender<ManagementCall>,
) -> Result<()> {
    stream.set_nodelay(true)?;
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    let hello = tokio::time::timeout(Duration::from_secs(5), read_json::<_, Hello>(&mut read))
        .await??
        .context("missing handshake")?;
    if hello.protocol != PROTOCOL
        || hello.plugin != "__manager__"
        || hello.token != token
        || hello.events
    {
        write_json(
            &mut write,
            &ServerMessage::Response {
                id: 0,
                result: Value::Null,
                error: Some(Fault {
                    code: "handshake_rejected".into(),
                    message: "invalid protocol or management credentials".into(),
                }),
            },
        )
        .await?;
        return Ok(());
    }
    write_json(
        &mut write,
        &ServerMessage::Response {
            id: 0,
            result: json!({"protocol":PROTOCOL}),
            error: None,
        },
    )
    .await?;
    let request = tokio::time::timeout(Duration::from_secs(5), read_json::<_, Request>(&mut read))
        .await??
        .context("missing request")?;
    let id = request.id;
    let (reply, response) = oneshot::channel();
    tokio::time::timeout(Duration::from_secs(3), send.send((request, reply)))
        .await?
        .context("supervisor shutting down")?;
    let result = tokio::time::timeout(Duration::from_secs(25), response)
        .await?
        .context("supervisor disconnected")?;
    let message = match result {
        Ok(result) => ServerMessage::Response {
            id,
            result,
            error: None,
        },
        Err(error) => {
            let fault = error.downcast_ref::<Fault>().cloned().unwrap_or(Fault {
                code: "management_error".into(),
                message: error.to_string(),
            });
            ServerMessage::Response {
                id,
                result: Value::Null,
                error: Some(fault),
            }
        }
    };
    write_json(&mut write, &message).await
}

async fn await_core(child: &mut Child, ready_path: &Path) -> Result<Ready> {
    let until = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("Core exited before ready: {status}")
        }
        if let Ok(bytes) = std::fs::read(ready_path) {
            if let Ok(ready) = serde_json::from_slice::<Ready>(&bytes) {
                if child.id() != Some(ready.pid) {
                    bail!("Core ready PID does not match owned child")
                }
                let address: std::net::SocketAddr = ready
                    .address
                    .parse()
                    .context("invalid Core ready address")?;
                if !address.ip().is_loopback() {
                    bail!("Core ready address must be loopback")
                }
                return Ok(ready);
            }
        }
        if tokio::time::Instant::now() >= until {
            bail!("Core readiness timed out")
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}
fn stop_signal() -> Result<impl std::future::Future<Output = Result<()>>> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut interrupt =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        Ok(async move {
            tokio::select! { _=term.recv()=>(), _=interrupt.recv()=>() }
            Ok(())
        })
    }
    #[cfg(not(unix))]
    {
        Ok(async move {
            match tokio::signal::ctrl_c().await {
                Ok(()) => Ok(()),
                Err(error) => {
                    eprintln!(
                        "[supervisor] console signal listener unavailable: {error}; use CLI stop"
                    );
                    std::future::pending::<Result<()>>().await
                }
            }
        })
    }
}

async fn run(config_path: PathBuf, state_path: PathBuf) -> Result<()> {
    let stop = stop_signal()?;
    tokio::pin!(stop);
    let config_path = std::fs::canonicalize(config_path).context("config file unavailable")?;
    let bytes = std::fs::read(&config_path)?;
    if bytes.len() > log_proto::MAX_WIRE {
        bail!("configuration exceeds 1 MiB")
    }
    let config: Config = serde_json::from_slice(&bytes).context("invalid configuration")?;
    let state_path = absolute(&state_path)?;
    let parent = state_path.parent().context("state parent")?;
    private_directory(parent)?;
    let state_file = private_file(&state_path, true, false).context("state already exists or cannot be reserved; stop its instance first; inspect a stale state before removing it")?;
    let directory = parent.join(format!(".log-print-runtime-{}", Uuid::new_v4()));
    let mut files = Files {
        state: state_path.clone(),
        directory: directory.clone(),
        file: Some(state_file),
    };
    private_directory(&directory)?;
    save_json(
        files.file.as_mut().expect("owned state file"),
        &json!({"starting":true,"pid":std::process::id()}),
    )?;
    let token = Uuid::new_v4().to_string();
    let mut plugin_tokens = BTreeMap::new();
    for plugin in &config.plugins {
        if plugin.id.starts_with("__")
            || plugin_tokens
                .insert(plugin.id.clone(), Uuid::new_v4().to_string())
                .is_some()
        {
            bail!("duplicate or reserved plugin identity: {}", plugin.id)
        }
    }
    let runtime = RuntimeConfig {
        config: config.clone(),
        admin_token: token.clone(),
        plugin_tokens: plugin_tokens.clone(),
    };
    let runtime_path = directory.join("config.json");
    save_json(&mut private_file(&runtime_path, true, false)?, &runtime)?;
    let ready_path = directory.join("core-ready.json");
    let config_dir = config_path.parent().context("config parent")?.to_path_buf();
    let core_binary = resolve_binary("log-print-core", &config_dir)?;
    let mut core = Command::new(core_binary)
        .arg("--runtime-config")
        .arg(&runtime_path)
        .arg("--ready-file")
        .arg(&ready_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("start log-print-core")?;
    eprintln!("[supervisor] spawned Core pid={:?}", core.id());
    let ready_result = tokio::select! {
        ready=await_core(&mut core,&ready_path)=>ready,
        signal=&mut stop=>signal.and_then(|_|Err(anyhow::anyhow!("startup interrupted"))),
    };
    let ready = match ready_result {
        Ok(ready) => ready,
        Err(e) => {
            let _ = core.start_kill();
            let _ = core.wait().await;
            return Err(e);
        }
    };
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(e) => {
            let _ = core.start_kill();
            let _ = core.wait().await;
            return Err(e.into());
        }
    };
    let state = State {
        protocol: PROTOCOL.into(),
        address: listener.local_addr()?.to_string(),
        token,
        pid: std::process::id(),
        core_pid: ready.pid,
        core_address: ready.address,
        config: config_path.display().to_string(),
        runtime_directory: directory.display().to_string(),
    };
    let core_stdin = core.stdin.take();
    let plugins = config
        .plugins
        .iter()
        .map(|spec| {
            (
                spec.id.clone(),
                Managed {
                    spec: spec.clone(),
                    child: None,
                    last_exit: None,
                    last_success: None,
                },
            )
        })
        .collect();
    let mut supervisor = Supervisor {
        state,
        config,
        config_dir,
        tokens: plugin_tokens,
        plugins,
        core,
        core_stdin,
        stopping: false,
    };
    let autostart: Vec<String> = supervisor
        .plugins
        .iter()
        .filter(|(_, p)| p.spec.autostart)
        .map(|(id, _)| id.clone())
        .collect();
    for id in &autostart {
        if let Err(error) = supervisor.start_plugin(id) {
            supervisor.cleanup(true).await;
            return Err(error);
        }
    }
    let plugin_startup = tokio::select! {
        result=supervisor.wait_plugins(&autostart)=>result,
        signal=&mut stop=>signal.and_then(|_|Err(anyhow::anyhow!("startup interrupted"))),
    };
    if let Err(error) = plugin_startup {
        supervisor.cleanup(true).await;
        return Err(error);
    }
    if let Err(error) = save_json(
        files.file.as_mut().expect("owned state file"),
        &supervisor.state,
    ) {
        supervisor.cleanup(true).await;
        return Err(error);
    }
    eprintln!(
        "[supervisor] ready pid={} core_pid={} state={}",
        supervisor.state.pid,
        supervisor.state.core_pid,
        state_path.display()
    );
    let (send, mut receive) = mpsc::channel::<ManagementCall>(32);
    let mut connections = JoinSet::new();
    let result: Result<()> = loop {
        tokio::select! {
            signal = &mut stop => { break signal; }
            exited = supervisor.core.wait() => { break Err(anyhow::anyhow!("Core exited unexpectedly: {:?}; stopping affected plugins", exited)); }
            accepted = listener.accept(), if connections.len() < 64 => {
                match accepted {
                    Ok((stream, _)) => { connections.spawn(management_connection(stream, supervisor.state.token.clone(), send.clone())); }
                    Err(error) => break Err(error.into()),
                }
            }
            _ = connections.join_next(), if !connections.is_empty() => (),
            Some((request, reply)) = receive.recv() => {
                let result = supervisor.handle(&request.op, request.args).await;
                let _ = reply.send(result);
                if supervisor.stopping { break Ok(()) }
            }
        }
    };
    let alive = supervisor.core.try_wait().ok().flatten().is_none();
    supervisor.cleanup(alive).await;
    // Allow the stop response to flush before dropping remaining authenticated sockets.
    let _ = tokio::time::timeout(Duration::from_millis(150), async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.abort_all();
    result
}

#[cfg(windows)]
fn prevent_inherited_caller_stdio() -> Result<()> {
    use windows_sys::Win32::Foundation::{
        GetHandleInformation, SetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    let mut seen = Vec::new();
    for kind in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        let handle = unsafe { GetStdHandle(kind) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error()).context("get caller standard handle");
        }
        if handle.is_null() || seen.contains(&handle) {
            continue;
        }
        seen.push(handle);
        let mut flags = 0;
        if unsafe { GetHandleInformation(handle, &mut flags) } == 0 {
            return Err(std::io::Error::last_os_error()).context("inspect caller standard handle");
        }
        if flags & HANDLE_FLAG_INHERIT != 0
            && unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0
        {
            return Err(std::io::Error::last_os_error())
                .context("prevent background inheritance of caller standard handle");
        }
    }
    Ok(())
}

async fn start(config: PathBuf, state: PathBuf) -> Result<()> {
    // Windows ordinary spawn inherits all inheritable handles, not only hStd*.
    // Keep the CLI's caller pipes out of the detached tree. Rust duplicates the
    // explicitly selected log/NUL handles for the child's own standard streams.
    #[cfg(windows)]
    prevent_inherited_caller_stdio()?;
    let state = absolute(&state)?;
    if state.exists() {
        bail!(
            "state {} already exists; inspect or stop that instance first",
            state.display()
        )
    }
    private_directory(state.parent().context("state parent")?)?;
    let stdout_path = log_path(&state, ".stdout.log");
    let stderr_path = log_path(&state, ".stderr.log");
    let stdout = private_file(&stdout_path, false, true)?;
    let stderr = private_file(&stderr_path, false, true)?;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--state")
        .arg(&state)
        .arg("run")
        .arg("--config")
        .arg(absolute(&config)?)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    command.process_group(0);
    #[cfg(windows)]
    command.creation_flags(0x00000008 | 0x00000200);
    let mut child = command.spawn().context("start background supervisor")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("supervisor exited: {status}; see {}", stderr_path.display())
        }
        if let Ok(bytes) = std::fs::read(&state) {
            if let Ok(saved) = serde_json::from_slice::<State>(&bytes) {
                if child.id() != Some(saved.pid) {
                    bail!("another supervisor acquired the state file")
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"started":true,"pid":saved.pid,"core_pid":saved.core_pid,"state":state,"stdout":stdout_path,"stderr":stderr_path})
                    )?
                );
                return Ok(());
            }
        }
        if tokio::time::Instant::now() >= deadline {
            // Leave an owned, still-starting supervisor intact so it can finish cleanup;
            // report its PID and log rather than kill a parent and orphan its children.
            bail!(
                "supervisor still starting pid={:?}; inspect {} and {}; it was not forcibly killed",
                child.id(),
                state.display(),
                stderr_path.display()
            )
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
async fn manager(state: &State, op: &str, args: Value) -> Result<Value> {
    rpc(&state.address, "__manager__", &state.token, op, args).await
}
fn read_has_gap(page: &Value) -> bool {
    page.get("gap")
        .is_some_and(|v| !v.is_null() && v != &Value::Bool(false))
        || page
            .get("gaps")
            .and_then(Value::as_array)
            .is_some_and(|v| !v.is_empty())
}
async fn read_page(state: &State, args: Value, wait_ms: u64) -> Result<Value> {
    let (client, _events, _controls) =
        log_plugin_sdk::Client::connect(&state.core_address, "__admin__", &state.token).await?;
    let mut page = client.request("read", args.clone()).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait_ms);
    loop {
        let records = page["records"]
            .as_array()
            .context("read reply has no records")?;
        if !records.is_empty() || read_has_gap(&page) || tokio::time::Instant::now() >= deadline {
            return Ok(page);
        }
        tokio::time::sleep_until(
            deadline.min(tokio::time::Instant::now() + Duration::from_millis(20)),
        )
        .await;
        if tokio::time::Instant::now() >= deadline {
            return Ok(page);
        }
        match tokio::time::timeout_at(deadline, client.request("read", args.clone())).await {
            Ok(result) => page = result?,
            // A read is side-effect free; retain the last actual snapshot on deadline.
            Err(_) => return Ok(page),
        }
    }
}
fn control(plugin: String, method: &str, args: Value) -> (String, Value) {
    (
        "core.call".into(),
        json!({"op":"control","args":{"target":plugin,"method":method,"args":args}}),
    )
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Action::Run { config } => return run(config, cli.state).await,
        Action::Start { config } => return start(config, cli.state).await,
        _ => (),
    }
    let state: State = serde_json::from_slice(&std::fs::read(&cli.state).with_context(|| {
        format!(
            "read instance state {}; start or run an instance first",
            cli.state.display()
        )
    })?)
    .context("instance state is not ready or is invalid")?;
    if state.protocol != PROTOCOL {
        bail!("incompatible state protocol: {}", state.protocol)
    }
    let is_stop = matches!(cli.command, Action::Stop);
    let mut raw = false;
    let mut read_wait = None;
    let (op, args) = match cli.command {
        Action::Status => ("status".into(), json!({})),
        Action::Streams => ("streams".into(), json!({})),
        Action::Read {
            stream,
            from,
            limit,
            epoch,
            wait_ms,
            raw: bytes,
        } => {
            raw = bytes;
            read_wait = Some(wait_ms);
            (
                "read".into(),
                json!({"stream":stream,"from":from,"limit":limit,"epoch":epoch}),
            )
        }
        Action::Config {
            plugin,
            command: None,
        } => ("config".into(), json!({"plugin":plugin})),
        Action::Config {
            command: Some(ConfigAction::Set { plugin, json }),
            ..
        } => control(plugin, "config.patch", parse_args(&json)?),
        Action::Plugin { command } => match command {
            PluginAction::Start { id } => ("plugin.start".into(), json!({"id":id})),
            PluginAction::Stop { id } => ("plugin.stop".into(), json!({"id":id})),
            PluginAction::Restart { id } => ("plugin.restart".into(), json!({"id":id})),
            PluginAction::Call { id, method, json } => control(id, &method, parse_args(&json)?),
        },
        Action::Session { command } => match command {
            SessionAction::List { plugin } => control(plugin, "sessions", json!({})),
            SessionAction::Create { plugin, json } => {
                control(plugin, "session.create", parse_args(&json)?)
            }
            SessionAction::Get { plugin, id } => control(plugin, "session.get", json!({"id":id})),
            SessionAction::Select { plugin, id } => {
                control(plugin, "session.select", json!({"id":id}))
            }
            SessionAction::Set {
                plugin,
                id,
                revision,
                json,
            } => control(
                plugin,
                "session.patch",
                json!({"id":id,"revision":revision,"patch":parse_args(&json)?}),
            ),
            SessionAction::Export {
                plugin,
                id,
                path,
                format,
                revision,
                width,
                height,
                overwrite,
            } => {
                let mut args = json!({"id":id,"path":absolute(&path)?.display().to_string(),"format":format,"overwrite":overwrite});
                if let Some(revision) = revision {
                    args["revision"] = json!(revision);
                }
                if let Some(width) = width {
                    args["width"] = json!(width);
                }
                if let Some(height) = height {
                    args["height"] = json!(height);
                }
                control(plugin, "session.export", args)
            }
        },
        Action::Call { op, json } => (
            "core.call".into(),
            json!({"op":op,"args":parse_args(&json)?}),
        ),
        Action::Stop => ("stop".into(), json!({})),
        _ => unreachable!(),
    };
    let result = if let Some(wait_ms) = read_wait {
        read_page(&state, args, wait_ms).await?
    } else {
        manager(&state, &op, args).await?
    };
    if raw {
        if read_has_gap(&result) {
            bail!("read reports a gap; inspect JSON output before extracting bytes")
        }
        let records = result["records"]
            .as_array()
            .context("read reply has no records")?;
        let mut out = std::io::stdout().lock();
        for record in records {
            let record: log_proto::Record = serde_json::from_value(record.clone())?;
            out.write_all(&record.payload)?;
        }
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    if is_stop {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while cli.state.exists() {
            if tokio::time::Instant::now() >= deadline {
                bail!(
                    "shutdown is still pending; state remains at {}",
                    cli.state.display()
                )
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
    Ok(())
}
