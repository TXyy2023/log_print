//! Real-process fixtures. All data is synthetic; Drop only cleans owned resources.
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, UdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};
use tempfile::TempDir;
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);
pub static CLEANUP_FAILED: AtomicBool = AtomicBool::new(false);
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}
pub fn bin(name: &str) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { root().join(p) })
        .unwrap_or_else(|| root().join("target"));
    target
        .join(std::env::var("LOG_PRINT_PROFILE").unwrap_or("debug".into()))
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}
pub fn fixture() -> String {
    std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}
pub fn s(v: &Value) -> String {
    v.as_str().expect("expected string").to_owned()
}
pub fn arr(v: &Value) -> &[Value] {
    v.as_array().expect("expected array")
}
pub fn text(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).unwrap_or_default()
}
pub fn bytes(path: impl AsRef<Path>) -> Vec<u8> {
    fs::read(path).unwrap_or_default()
}
pub fn write_json(path: impl AsRef<Path>, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
pub fn read_json(path: impl AsRef<Path>) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}
pub fn pause(ms: u64) {
    thread::sleep(Duration::from_millis(ms));
}
pub fn eventually<T>(seconds: u64, mut check: impl FnMut() -> Option<T>) -> T {
    let start = Instant::now();
    loop {
        assert!(!INTERRUPTED.load(Ordering::Relaxed), "interrupted");
        if let Some(v) = check() {
            return v;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "condition not reached in {seconds}s"
        );
        pause(25);
    }
}
pub fn until(mut check: impl FnMut() -> bool) {
    eventually(8, || check().then_some(()));
}
pub fn payload(rows: &Value, channel: Option<&str>) -> Vec<u8> {
    arr(rows)
        .iter()
        .filter(|r| channel.is_none_or(|c| r["channel"] == c))
        .flat_map(|r| arr(&r["payload"]).iter().map(|b| b.as_u64().unwrap() as u8))
        .collect()
}
pub struct Process(pub Child);
impl Process {
    pub fn spawn(cmd: &mut Command) -> Self {
        Self(cmd.spawn().unwrap())
    }
    pub fn exited(&mut self) -> Option<i32> {
        self.0.try_wait().unwrap().map(|s| s.code().unwrap_or(-1))
    }
    pub fn wait(&mut self, seconds: u64) -> i32 {
        eventually(seconds, || self.exited())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
/// Drain PIPEs concurrently, preserving binary output while bounding child lifetime.
pub fn capture(cmd: &mut Command, seconds: u64) -> Output {
    let mut child = Process::spawn(
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    );
    let mut out = child.0.stdout.take().unwrap();
    let mut err = child.0.stderr.take().unwrap();
    let stdout = thread::spawn(move || {
        let mut b = Vec::new();
        out.read_to_end(&mut b).unwrap();
        b
    });
    let stderr = thread::spawn(move || {
        let mut b = Vec::new();
        err.read_to_end(&mut b).unwrap();
        b
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.0.try_wait().unwrap() {
            break s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "command timed out: {cmd:?}"
        );
        pause(25);
    };
    until(|| stdout.is_finished() && stderr.is_finished());
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}
pub struct Rpc {
    tcp: Option<BufReader<TcpStream>>,
    udp: Option<UdpSocket>,
    seq: u64,
    pub welcome: Value,
}
impl Rpc {
    pub fn connect(
        address: &str,
        plugin: &str,
        token: &str,
        transport: &str,
        events: bool,
        protocol: &str,
    ) -> anyhow::Result<Self> {
        let mut r = Self {
            tcp: None,
            udp: None,
            seq: 0,
            welcome: Value::Null,
        };
        if transport == "udp" {
            let sock = UdpSocket::bind("127.0.0.1:0")?;
            sock.connect(address)?;
            socket2::SockRef::from(&sock).set_send_buffer_size(256 * 1024)?;
            sock.set_read_timeout(Some(Duration::from_secs(4)))?;
            r.udp = Some(sock);
        } else {
            let sock = TcpStream::connect_timeout(&address.parse()?, Duration::from_secs(4))?;
            sock.set_read_timeout(Some(Duration::from_secs(4)))?;
            sock.set_write_timeout(Some(Duration::from_secs(4)))?;
            sock.set_nodelay(true)?;
            r.tcp = Some(BufReader::new(sock));
        }
        r.send(&json!(
            { "protocol" : protocol, "plugin" : plugin, "token" : token, "events" :
            events }
        ))?;
        r.welcome = Self::result(r.receive()?)?;
        Ok(r)
    }
    pub fn send(&mut self, v: &Value) -> anyhow::Result<()> {
        let mut data = serde_json::to_vec(v)?;
        if let Some(t) = &mut self.tcp {
            data.push(b'\n');
            t.get_mut().write_all(&data)?;
        } else {
            self.udp.as_ref().unwrap().send(&data)?;
        }
        Ok(())
    }
    pub fn receive(&mut self) -> anyhow::Result<Value> {
        let mut data = Vec::new();
        if let Some(t) = &mut self.tcp {
            t.take(1024 * 1024 + 1).read_until(b'\n', &mut data)?;
        } else {
            data.resize(65536, 0);
            let n = self.udp.as_ref().unwrap().recv(&mut data)?;
            data.truncate(n)
        }
        anyhow::ensure!(
            !data.is_empty() && data.len() <= 1024 * 1024,
            "EOF or oversized frame"
        );
        Ok(serde_json::from_slice(&data)?)
    }
    fn result(v: Value) -> anyhow::Result<Value> {
        anyhow::ensure!(v["error"].is_null(), "{}", v["error"]);
        Ok(v["result"].clone())
    }
    pub fn try_call(&mut self, op: &str, args: Value) -> anyhow::Result<Value> {
        self.seq += 1;
        self.send(&json!({ "id" : self.seq, "op" : op, "args" : args }))?;
        let m = self.receive()?;
        anyhow::ensure!(m["type"] == "response" && m["id"] == self.seq, "{m}");
        Self::result(m)
    }
    pub fn call(&mut self, op: &str, args: Value) -> Value {
        self.try_call(op, args).unwrap()
    }
    pub fn publish(&mut self, stream: &str, data: &[u8], extra: Value) -> Value {
        let mut args = json!(
            { "stream" : stream, "payload" : data, "key" : "same-key" }
        );
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        if self.udp.is_some() {
            self.send(&json!({ "id" : 0, "op" : "publish", "args" : args }))
                .unwrap();
            Value::Null
        } else {
            self.call("publish", args)
        }
    }
    pub fn timeout(&self, ms: u64) {
        if let Some(t) = &self.tcp {
            t.get_ref()
                .set_read_timeout(Some(Duration::from_millis(ms)))
                .unwrap()
        } else {
            self.udp
                .as_ref()
                .unwrap()
                .set_read_timeout(Some(Duration::from_millis(ms)))
                .unwrap()
        }
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        if self.udp.is_some() {
            let _ = self.send(&json!({ "id" : 0, "op" : "disconnect", "args" : {} }));
        }
    }
}
pub struct Core {
    gated: bool,
    pub temp: TempDir,
    pub transport: String,
    pub address: String,
    pub process: Option<Process>,
}
impl Core {
    pub fn new(transport: &str, options: Value, plugins: Value) -> Self {
        Self::configured(transport, options, plugins, false)
    }
    pub fn gated(options: Value, plugins: Value) -> Self {
        Self::configured("tcp", options, plugins, true)
    }
    fn configured(transport: &str, options: Value, plugins: Value, gated: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut opts = options;
        opts["transport"] = json!(transport);
        let tokens: serde_json::Map<String, Value> = arr(&plugins)
            .iter()
            .map(|p| (s(&p["id"]), p["id"].clone()))
            .collect();
        write_json(
            temp.path().join("runtime.json"),
            &json!(
                { "config" : { "core" : opts, "plugins" : plugins }, "admin_token" :
                "admin", "plugin_tokens" : tokens }
            ),
        );
        let mut c = Self {
            gated,
            temp,
            transport: transport.into(),
            address: String::new(),
            process: None,
        };
        c.start();
        c
    }
    pub fn default() -> Self {
        Self::new(
            "tcp",
            json!({}),
            json!(
                [{ "id" : "a", "role" : "input", "bin" : "unused", "streams" : [{ "id" :
                "raw", "description" : "stream A" }] }, { "id" : "b", "role" : "input",
                "bin" : "unused", "streams" : [{ "id" : "other", "description" :
                "stream B" }] }, { "id" : "out", "role" : "output", "bin" : "unused",
                "reads" : ["raw", "other"] }, { "id" : "other-out", "role" : "output",
                "bin" : "unused", "reads" : ["raw"] }]
            ),
        )
    }
    pub fn path(&self) -> &Path {
        self.temp.path()
    }
    pub fn start(&mut self) {
        let ready = self.path().join("ready.json");
        let _ = fs::remove_file(&ready);
        let log = File::options()
            .create(true)
            .append(true)
            .open(self.path().join("core.log"))
            .unwrap();
        let mut command = if self.gated {
            let mut command = Command::new(fixture());
            command.args(["fixture", "gated-core", self.path().to_str().unwrap()]);
            command
        } else {
            let mut command = Command::new(bin("log-print-core"));
            command.args([
                "--runtime-config",
                self.path().join("runtime.json").to_str().unwrap(),
                "--ready-file",
                ready.to_str().unwrap(),
            ]);
            command
        };
        self.process = Some(Process::spawn(
            command
                .stdin(Stdio::piped())
                .stdout(log.try_clone().unwrap())
                .stderr(log),
        ));
        self.address = s(&eventually(8, || {
            assert!(
                self.process.as_mut().unwrap().exited().is_none(),
                "{}",
                text(self.path().join("core.log"))
            );
            read_json(&ready)
        })["address"]);
    }
    pub fn stop(&mut self) {
        if let Some(mut p) = self.process.take() {
            p.0.stdin.take();
            assert_eq!(p.wait(5), 0, "Core failed on parent stdin EOF")
        }
    }
    pub fn rpc(&self, id: &str) -> Rpc {
        self.events(id, false)
    }
    pub fn events(&self, id: &str, events: bool) -> Rpc {
        Rpc::connect(
            &self.address,
            id,
            if id == "__admin__" { "admin" } else { id },
            &self.transport,
            events,
            "log-print/2",
        )
        .unwrap()
    }
    pub fn admin(&self) -> Rpc {
        self.rpc("__admin__")
    }
    pub fn streams(&self) -> Value {
        self.admin().call("status", json!({}))["streams"].clone()
    }
    pub fn stream(&self, owner: &str) -> String {
        s(&arr(&self.streams())
            .iter()
            .find(|v| v["owner"] == owner)
            .unwrap()["id"])
    }
    pub fn records(&self, owner: &str) -> Value {
        self.admin().call(
            "read",
            json!({ "stream" : self.stream(owner), "limit" : 64 }),
        )["records"]
            .clone()
    }
}
impl Drop for Core {
    fn drop(&mut self) {
        if let Some(p) = &mut self.process {
            p.0.stdin.take();
            let start = Instant::now();
            while p.exited().is_none() && start.elapsed() < Duration::from_secs(5) {
                pause(25)
            }
            if p.exited().is_none() {
                eprintln!("Core failed to exit on parent stdin EOF");
                CLEANUP_FAILED.store(true, Ordering::Relaxed);
            }
        }
        self.process.take();
    }
}
pub struct Plugin<'a> {
    pub core: &'a Core,
    pub id: String,
    pub process: Process,
    pub stdout: PathBuf,
    pub stderr: PathBuf,
}
impl<'a> Plugin<'a> {
    pub fn new(core: &'a Core, spec: &Value, env: &[(&str, &str)]) -> Self {
        let id = s(&spec["id"]);
        let stdout = core.path().join(format!("{id}.stdout"));
        let stderr = core.path().join(format!("{id}.stderr"));
        let binary = s(&spec["bin"]);
        let binary = if Path::new(&binary).is_absolute() {
            PathBuf::from(binary)
        } else {
            bin(&binary)
        };
        let process = Process::spawn(
            Command::new(binary)
                .env("LOG_PRINT_CORE", &core.address)
                .env("LOG_PRINT_PLUGIN", &id)
                .env("LOG_PRINT_TOKEN", &id)
                .env("LOG_PRINT_TRANSPORT", &core.transport)
                .env("LOG_PRINT_CONFIG", spec["config"].to_string())
                .envs(env.iter().copied())
                .stdin(Stdio::null())
                .stdout(File::create(&stdout).unwrap())
                .stderr(File::create(&stderr).unwrap()),
        );
        Self {
            core,
            id,
            process,
            stdout,
            stderr,
        }
    }
    pub fn report(&self) -> Value {
        self.core
            .admin()
            .call("plugin.status", json!({ "plugin" : self.id }))["report"]
            .clone()
    }
    pub fn ready(&mut self, state: &str) -> Value {
        eventually(8, || {
            let r = self.report();
            if r["state"] == state {
                Some(r)
            } else {
                assert!(
                    self.process.exited().is_none(),
                    "plugin exited before {state}: {} {r}",
                    text(&self.stderr)
                );
                None
            }
        })
    }
    pub fn wait(&mut self, success: bool) -> Value {
        let code = self.process.wait(10);
        assert_eq!(code == 0, success, "{}", text(&self.stderr));
        self.report()
    }
    pub fn stop(&mut self) -> Value {
        let v = self.core.admin().call(
            "control",
            json!({ "target" : self.id, "method" : "shutdown" }),
        );
        self.wait(true);
        v
    }
}
impl Drop for Plugin<'_> {
    fn drop(&mut self) {
        if self.process.exited().is_none() {
            let _ = self.core.admin().try_call(
                "control",
                json!({ "target" : self.id, "method" : "shutdown" }),
            );
            let t = Instant::now();
            while self.process.exited().is_none() && t.elapsed() < Duration::from_secs(8) {
                pause(25)
            }
        }
    }
}
pub struct App {
    pub temp: TempDir,
    pub state: PathBuf,
    pub config: PathBuf,
    pub options: Option<Vec<String>>,
    pub allow_cleanup_failure: bool,
}
impl App {
    pub fn new(config: Value) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        write_json(&path, &config);
        Self {
            state: temp.path().join("state.json"),
            temp,
            config: path,
            options: None,
            allow_cleanup_failure: false,
        }
    }
    pub fn configured(config: Value) -> Self {
        let a = Self::new(config);
        a.start();
        a
    }
    pub fn native(options: &[&str]) -> Self {
        let mut a = Self::new(json!({}));
        fs::remove_file(&a.config).unwrap();
        a.options = Some(options.iter().map(|s| s.to_string()).collect());
        a.start();
        a
    }
    pub fn start(&self) {
        let mut args = vec!["start".to_string()];
        if let Some(o) = &self.options {
            args.extend(o.clone())
        } else {
            args.extend([
                "--config".into(),
                self.config.to_string_lossy().into_owned(),
            ])
        }
        let v = self.cli(&args.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(v.starts_with("Started.\n"), "{v}");
    }
    pub fn raw(&self, args: &[&str]) -> Output {
        capture(
            Command::new(bin("log-print"))
                .arg("--state")
                .arg(&self.state)
                .args(args)
                .current_dir(root()),
            45,
        )
    }
    pub fn cli(&self, args: &[&str]) -> String {
        let r = self.raw(args);
        assert!(
            r.status.success(),
            "{args:?}: {} {}",
            String::from_utf8_lossy(&r.stdout),
            String::from_utf8_lossy(&r.stderr)
        );
        String::from_utf8(r.stdout).unwrap()
    }
    pub fn manager(&self) -> Rpc {
        let v = read_json(&self.state).unwrap();
        Rpc::connect(
            &s(&v["address"]),
            "__manager__",
            &s(&v["token"]),
            "tcp",
            false,
            "log-print/2",
        )
        .unwrap()
    }
    pub fn inspect(&self, command: &str, arg: Option<&str>) -> Value {
        match command {
            "status" | "streams" | "config" => self.manager().call(
                command,
                arg.map(|p| json!({ "plugin" : p })).unwrap_or(json!({})),
            ),
            "stream" | "read" => self.manager().call(
                "core.call",
                json!(
                    { "op" : if command == "stream" { "stream.get" } else {
                    "read" }, "args" : { "stream" : arg.unwrap() } }
                ),
            ),
            _ => panic!("unknown inspection"),
        }
    }
    pub fn control(&self, id: &str, method: &str, args: Value) -> Value {
        self.manager().call(
            "core.call",
            json!(
                { "op" : "control", "args" : { "target" : id, "method" : method,
                "args" : args } }
            ),
        )
    }
    pub fn stop(&self) {
        let v = self.cli(&["stop"]);
        assert!(v.contains("success: yes"), "{v}");
        assert!(!self.state.exists())
    }
}
impl Drop for App {
    fn drop(&mut self) {
        if self.state.exists() {
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.raw(&["stop"])));
            let clean = result.is_ok_and(|r| {
                (r.status.success() || self.allow_cleanup_failure) && !self.state.exists()
            });
            if !clean {
                eprintln!("owned application cleanup failed: {}", self.state.display());
                CLEANUP_FAILED.store(true, Ordering::Relaxed);
            }
        }
    }
}
