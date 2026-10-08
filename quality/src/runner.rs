use crate::support::*;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{atomic::Ordering, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub fn stamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .to_string()
}
pub fn redact(text: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
            regex::Regex::new(
                    r#"(?i)((?:["']?)(?:token|password|secret|authorization)(?:["']?)\s*[:=]\s*)(?:["'][^"']*(?:["']|$)|[^\s,}]+)"#,
                )
                .unwrap()
        })
        .replace_all(text, "${1}<redacted>")
        .into_owned()
}
pub fn frontend() -> Result<()> {
    let dir = root().join("project/plugins/outputs/output-webui/frontend");
    for args in [vec!["ci"], vec!["run", "build"]] {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.args(["/d", "/s", "/c", "npm.cmd"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = Command::new("npm");
        let r = capture(cmd.args(args).current_dir(&dir), 1800);
        print!("{}", String::from_utf8_lossy(&r.stdout));
        eprint!("{}", String::from_utf8_lossy(&r.stderr));
        anyhow::ensure!(r.status.success(), "frontend command failed");
    }
    Ok(())
}
fn steps(exe: &str) -> Vec<(String, Vec<String>)> {
    let mut v = vec![
        ("frontend", vec![exe, "frontend"]),
        ("fmt", vec!["cargo", "fmt", "--all", "--check"]),
        (
            "clippy",
            vec![
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        ),
        (
            "rust-tests",
            vec!["cargo", "test", "--workspace", "--locked"],
        ),
        // The coordinator is already built. Do not replace a running .exe on Windows.
        (
            "build",
            vec![
                "cargo",
                "build",
                "--workspace",
                "--exclude",
                "log-print-quality",
                "--examples",
                "--bins",
                "--locked",
            ],
        ),
    ];
    for name in [
        "protocol-v2",
        "supervisor-v2",
        "supervisor-failures-v2",
        "cli-v2",
        "inputs-v2",
        "outputs-v2",
        "webui-v2",
        "tui-v2",
    ] {
        v.push((name, vec![exe, "suite", name]));
    }
    v.into_iter()
        .map(|(n, c)| (n.into(), c.into_iter().map(str::to_owned).collect()))
        .collect()
}
fn isolate(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP);
    }
}
fn stop(p: &mut Process) {
    if p.exited().is_some() {
        return;
    }
    #[cfg(unix)]
    unsafe {
        libc::kill(-(p.0.id() as i32), libc::SIGINT);
    }
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent(
            windows_sys::Win32::System::Console::CTRL_BREAK_EVENT,
            p.0.id(),
        );
    }
    let t = Instant::now();
    while p.exited().is_none() && t.elapsed() < Duration::from_secs(40) {
        pause(25)
    }
    if p.exited().is_none() {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(p.0.id() as i32), libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &p.0.id().to_string(), "/T", "/F"])
                .output();
        }
        let _ = p.0.kill();
        let _ = p.0.wait();
    }
}
struct Tail {
    reader: File,
    pending: Vec<u8>,
    discard: bool,
}
impl Tail {
    fn new(path: &Path) -> Result<Self> {
        Ok(Self {
            reader: File::open(path)?,
            pending: vec![],
            discard: false,
        })
    }
    fn poll(&mut self, name: &str) -> Result<()> {
        let mut buf = [0; 65536];
        for _ in 0..4 {
            let count = self.reader.read(&mut buf)?;
            if count == 0 {
                break;
            }
            for &byte in &buf[..count] {
                if self.discard {
                    if byte == b'\n' {
                        self.discard = false;
                    }
                    continue;
                }
                if byte == b'\n' {
                    println!(
                        "[{name}] {}",
                        redact(&String::from_utf8_lossy(&self.pending))
                    );
                    self.pending.clear();
                } else {
                    self.pending.push(byte);
                    if self.pending.len() > 65536 {
                        println!("[{name}] long line retained in artifact");
                        self.pending.clear();
                        self.discard = true;
                    }
                }
            }
        }
        Ok(())
    }
    fn finish(&mut self, name: &str) {
        if !self.pending.is_empty() {
            println!(
                "[{name}] {}",
                redact(&String::from_utf8_lossy(&self.pending))
            );
            self.pending.clear();
        }
    }
}
fn execute(name: &str, command: &[String], directory: &Path, timeout: u64) -> Value {
    let before = Instant::now();
    let path = directory.join(format!("{name}.log"));
    let started = stamp();
    println!("RUN {name}: {command:?}");
    let result = (|| -> Result<(i32, &str)> {
        let log = File::create(&path)?;
        let mut cmd = Command::new(&command[0]);
        cmd.args(&command[1..])
            .current_dir(root())
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        isolate(&mut cmd);
        let mut p = Process(cmd.spawn()?);
        let mut tail = Tail::new(&path)?;
        loop {
            tail.poll(name)?;
            if let Some(code) = p.exited() {
                while tail.reader.metadata()?.len()
                    > std::io::Seek::stream_position(&mut tail.reader)?
                {
                    tail.poll(name)?;
                }
                tail.finish(name);
                return Ok((code, if code == 0 { "pass" } else { "fail" }));
            }
            if INTERRUPTED.load(Ordering::Relaxed) {
                stop(&mut p);
                return Ok((130, "interrupted"));
            }
            if before.elapsed() >= Duration::from_secs(timeout) {
                stop(&mut p);
                return Ok((124, "fail"));
            }
            pause(100)
        }
    })();
    let (code, status, error) = match result {
        Ok((code, status)) => (
            code,
            status,
            if code == 124 {
                Some("timeout; requested owned harness cleanup".to_owned())
            } else {
                None
            },
        ),
        Err(e) => (127, "fail", Some(e.to_string())),
    };
    let v = json!(
        { "name" : name, "command" : command, "cwd" : root(), "started_unix_ms" :
        started, "log" : format!("{name}.log"), "timeout_seconds" : timeout, "returncode"
        : code, "status" : status, "error" : error, "seconds" : before.elapsed()
        .as_secs_f64() }
    );
    println!(
        "{} {name}: code={code}, {:.3}s, log={}",
        status.to_uppercase(),
        before.elapsed().as_secs_f64(),
        path.display()
    );
    v
}
fn probe(cmd: &[&str]) -> Value {
    match std::panic::catch_unwind(|| {
        capture(Command::new(cmd[0]).args(&cmd[1..]).current_dir(root()), 20)
    }) {
        Ok(r) => {
            json!(
                { "command" : cmd, "returncode" : r.status.code(), "stdout" :
                String::from_utf8_lossy(& r.stdout).trim(), "stderr" :
                String::from_utf8_lossy(& r.stderr).trim() }
            )
        }
        Err(_) => json!({ "command" : cmd, "error" : "probe failed" }),
    }
}
pub fn run(report: Option<PathBuf>, timeout: u64, skip_build: bool, list: bool) -> Result<bool> {
    let exe = std::env::current_exe()?;
    let steps = steps(
        exe.to_str()
            .context("quality executable path must be UTF-8")?,
    );
    if list {
        for (n, c) in steps {
            println!("{n}: {c:?}")
        }
        return Ok(true);
    }
    let directory = report.unwrap_or_else(|| {
        root().join("quality/artifacts/validation").join(format!(
            "{}-{}-{}",
            stamp(),
            std::env::consts::OS,
            std::process::id()
        ))
    });
    let directory = std::path::absolute(directory)?;
    if let Some(parent) = directory.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(&directory).context("report directory must be new")?;
    let worker = directory.join(format!("quality-worker{}", std::env::consts::EXE_SUFFIX));
    fs::copy(&exe, &worker)?;
    let mut selected = steps.clone();
    for (_, cmd) in &mut selected {
        if cmd[0] == exe.to_string_lossy() {
            cmd[0] = worker.to_string_lossy().into_owned();
        }
    }
    let environment = json!(
        { "timestamp_unix_ms" : stamp(), "system" : std::env::consts::OS, "architecture"
        : std::env::consts::ARCH, "rustc" : probe(& ["rustc", "-Vv"]), "cargo" : probe(&
        ["cargo", "-V"]), "node" : probe(& ["node", "--version"]), "git_head" : probe(&
        ["git", "rev-parse", "HEAD"]), "ci" : std::env::vars().filter(| (k, _) | ["CI",
        "GITHUB_ACTIONS", "RUNNER_OS", "RUNNER_ARCH", "LOG_PRINT_PROFILE"].contains(& k
        .as_str())).collect:: < std::collections::BTreeMap < _, _ >> () }
    );
    write_json(directory.join("environment.json"), &environment);
    let mut results = vec![];
    let mut blocked = false;
    for (name, command) in &selected {
        if skip_build && ["fmt", "clippy", "rust-tests", "build"].contains(&name.as_str()) {
            results.push(json!(
                { "name" : name, "status" : "skip", "reason" :
                "--skip-build is iteration only" }
            ));
            continue;
        }
        if blocked {
            results.push(json!(
                { "name" : name, "status" : "skip", "reason" :
                "frontend or workspace build failed; stale assets/binaries must not be accepted"
                }
            ));
            continue;
        }
        let limit = if name == "supervisor-v2" {
            timeout.min(120)
        } else {
            timeout
        };
        let result = execute(name, command, &directory, limit);
        let interrupted = result["status"] == "interrupted";
        if ["frontend", "build"].contains(&name.as_str()) && result["status"] != "pass" {
            blocked = true;
        }
        results.push(result);
        write_json(
            directory.join("results.json"),
            &json!({ "environment" : environment, "results" : results }),
        );
        if interrupted {
            break;
        }
    }
    let failed = results
        .iter()
        .any(|r| r["status"] == "fail" || r["status"] == "interrupted");
    let complete = !skip_build
        && !failed
        && results.len() == steps.len()
        && results.iter().all(|r| r["status"] == "pass");
    let boundaries = [
        "This host only; does not establish other-platform execution.",
        "No real-device, power-loss, or benchmark results are inferred from this suite.",
    ];
    write_json(
        directory.join("results.json"),
        &json!(
            { "environment" : environment, "complete_selected_suite" : complete,
            "iteration_only" : skip_build, "results" : results, "optional" : [],
            "boundaries" : boundaries }
        ),
    );
    let mut summary = format!(
        "# Validation result\n\nPlatform: {}\n\nComplete selected suite: {complete}\n\n| Step | Status | Exit | Seconds | Log |\n|---|---|---:|---:|---|\n",
        std::env::consts::OS
    );
    for row in &results {
        summary.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            s(&row["name"]),
            s(&row["status"]),
            row["returncode"],
            row["seconds"],
            row["log"]
                .as_str()
                .unwrap_or_else(|| row["reason"].as_str().unwrap_or(""))
        ));
    }
    summary.push_str(&format!("\n{}\n", boundaries.join("\n")));
    fs::write(directory.join("summary.md"), summary)?;
    let _ = fs::remove_file(worker);
    println!("REPORT {}", directory.join("results.json").display());
    Ok(!failed)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_command_is_recorded_as_failure() {
        let td = tempfile::tempdir().unwrap();
        let missing = td
            .path()
            .join("missing-executable")
            .to_string_lossy()
            .into_owned();
        let row = execute("missing", &[missing], td.path(), 1);
        assert_eq!(row["status"], "fail");
        assert_eq!(row["returncode"], 127);
        assert!(td.path().join("missing.log").exists());
    }

    #[test]
    fn report_directory_is_never_overwritten() {
        let td = tempfile::tempdir().unwrap();
        let evidence = td.path().join("results.json");
        fs::write(&evidence, "previous evidence").unwrap();
        assert!(run(Some(td.path().to_owned()), 1, true, false).is_err());
        assert_eq!(text(evidence), "previous evidence");
    }

    #[test]
    fn console_redacts_values() {
        assert_eq!(
            redact(r#"token="private value" password: secret authorization='Bearer 123'"#),
            "token=<redacted> password: <redacted> authorization=<redacted>"
        );
    }
    #[test]
    fn oversized_lines_are_bounded() {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("log");
        fs::write(&p, vec![b'x'; 1_000_000]).unwrap();
        let mut tail = Tail::new(&p).unwrap();
        for _ in 0..4 {
            tail.poll("test").unwrap();
            assert!(tail.pending.len() <= 65536)
        }
        assert!(tail.discard);
    }
}
