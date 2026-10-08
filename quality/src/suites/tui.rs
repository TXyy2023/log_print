use super::webui::request;
use crate::{suites::Suite, support::*};
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    sync::mpsc,
};
struct Terminal {
    process: Process,
    lines: mpsc::Receiver<String>,
    _errors: std::thread::JoinHandle<String>,
}
impl Terminal {
    fn new(url: &str) -> Self {
        let mut process = Process::spawn(
            Command::new(bin("examples/pty_driver"))
                .arg(bin("output-tui"))
                .arg(url)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .current_dir(root()),
        );
        let stdout = process.0.stdout.take().unwrap();
        let stderr = process.0.stderr.take().unwrap();
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        let errors = std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            BufReader::new(stderr).read_to_string(&mut s).unwrap();
            s
        });
        let term = Self {
            process,
            lines,
            _errors: errors,
        };
        assert_eq!(term.read()["ready"], true);
        term
    }
    fn read(&self) -> Value {
        serde_json::from_str(
            &self
                .lines
                .recv_timeout(std::time::Duration::from_secs(25))
                .expect("PTY driver reply timed out/disconnected"),
        )
        .unwrap()
    }
    fn call(&mut self, args: Value) -> Value {
        writeln!(self.process.0.stdin.as_mut().unwrap(), "{args}").unwrap();
        self.process.0.stdin.as_mut().unwrap().flush().unwrap();
        self.read()
    }
    fn finish(&mut self, send: &str) {
        let r = self.call(json!({ "send" : send, "finish" : true }));
        assert_eq!(r["exit"], 0);
        assert_eq!(r["alternate"], false);
        #[cfg(unix)]
        assert_eq!(r["restored"], true);
        self.process.0.stdin.take();
        assert_eq!(self.process.wait(12), 0);
    }
}
pub fn run(t: &mut Suite) {
    t.case(
        "attach_keys_mouse_resize_conflict_detach_and_webui_sync",
        || {
            let a = App::native(&["--output-webui", "web"]);
            let url = s(&a.control("web", "url", json!({}))["url"]);
            a.cli(&[
                "tui",
                "web",
                "page",
                "set",
                "--sidebar-open",
                "false",
                "--view-x",
                "0",
                "--view-y",
                "0",
                "--theme",
                "dark",
            ]);
            a.cli(&[
                "tui",
                "web",
                "panel",
                "add",
                "--title",
                "PTY console",
                "--left",
                "0",
                "--top",
                "0",
                "--panel-width",
                "640",
                "--panel-height",
                "320",
            ]);
            let panel = s(&request(&url, "", json!({}))["pages"][0]["panels"][0]["id"]);
            a.cli(&["tui", "web", "page", "set", "--active-panel", &panel]);
            let mut term = Terminal::new(&url);
            assert_eq!(
                term.call(json!({ "wait" : "PTY console" }))["alternate"],
                true
            );
            term.call(json!({ "send" : "?", "wait" : "TERMINAL WORKBENCH" }));
            term.call(json!({ "send" : "\u{1b}", "absent" : "TERMINAL WORKBENCH" }));
            term.call(json!(
                { "send" : ":panel set --title \"终端设置\"\r", "wait" :
                "Saved · panel.set" }
            ));
            until(|| request(&url, "", json!({}))["pages"][0]["panels"][0]["title"] == "终端设置");
            term.call(json!({ "send" : "/stale-filter", "wait" : "Text filter" }));
            a.cli(&[
                "webui", "web", "panel", "set", "--panel", &panel, "--text", "from-cli",
            ]);
            term.call(json!({ "send" : "\r", "wait" : "revision_conflict" }));
            assert_eq!(
                request(&url, "", json!({}))["pages"][0]["panels"][0]["text"],
                "from-cli"
            );
            term.call(json!(
                { "send" : ":panel set --text \"\"\r", "wait" : "Saved · panel.set"
                }
            ));
            term.call(json!({ "send" : "m\u{1b}[C\r" }));
            until(|| request(&url, "", json!({}))["pages"][0]["panels"][0]["left"] == 8.0);
            term.call(json!(
                { "wait" : format!("rev {}", request(& url, "", json!({}))
                ["revision"]) }
            ));
            term.call(json!({ "send" : "\u{1b}[<0;10;4M\u{1b}[<32;14;6M\u{1b}[<0;14;6m" }));
            until(|| request(&url, "", json!({}))["pages"][0]["panels"][0]["left"] == 40.0);
            assert!(s(
                &term.call(json!({ "resize" : [30, 10], "wait" : "Terminal too small"
                }))["screen"]
            )
            .contains("Terminal too small"));
            term.call(json!({ "resize" : [140, 36], "wait" : "终端设置" }));
            let mut mirror = Terminal::new(&url);
            mirror.call(json!({ "wait" : "终端设置" }));
            a.cli(&["tui", "web", "page", "set", "--title", "SYNCHRONIZED"]);
            term.call(json!({ "wait" : "SYNCHRONIZED" }));
            mirror.call(json!({ "wait" : "SYNCHRONIZED" }));
            mirror.finish("\u{3}");
            term.finish("q");
            assert_eq!(
                request(&url, "", json!({}))["pages"][0]["title"],
                "SYNCHRONIZED"
            );
            let r = a.raw(&["tui", "web", "attach"]);
            assert!(!r.status.success());
            assert!(String::from_utf8_lossy(&r.stderr).contains("interactive terminal"));
            let snapshot = a.cli(&[
                "tui",
                "web",
                "attach",
                "--snapshot",
                "--width",
                "140",
                "--height",
                "36",
            ]);
            assert!(snapshot.contains("SYNCHRONIZED") && !snapshot.contains('\u{1b}'));
        },
    );
    t.case(
        "tui_archive_snapshot_history_and_connection_failure",
        || {
            let td = tempfile::tempdir().unwrap();
            let source = td.path().join("source.log");
            fs::write(&source, "temperature=20\nERROR 早期记录\n").unwrap();
            let a = App::native(&[
                "--input-file",
                &format!("source={}", source.display()),
                "--set",
                "source.from_start=true",
                "--output-tui",
                "term",
                "--tui-archive",
                &format!("term={}/capture", td.path().display()),
            ]);
            let url = s(&a.control("term", "url", json!({}))["url"]);
            a.cli(&[
                "tui",
                "term",
                "page",
                "set",
                "--sidebar-open",
                "false",
                "--view-x",
                "0",
                "--view-y",
                "0",
            ]);
            a.cli(&[
                "tui",
                "term",
                "panel",
                "add",
                "--title",
                "Archived logs",
                "--left",
                "0",
                "--top",
                "0",
                "--panel-width",
                "900",
                "--panel-height",
                "400",
                "--column",
                "text",
            ]);
            let mut term = Terminal::new(&url);
            term.call(json!({ "wait" : "ERROR 早期记录" }));
            term.call(json!({ "send" : "h", "wait" : "HISTORY" }));
            term.call(json!({ "send" : "o", "wait" : "archive_and_memory" }));
            term.call(json!(
                { "send" : "\u{1b}", "absent" : "Coverage / gaps / query progress" }
            ));
            term.call(json!({ "send" : "l", "wait" : "LIVE" }));
            term.call(json!({ "send" : " ", "wait" : "PAUSED" }));
            fs::OpenOptions::new()
                .append(true)
                .open(&source)
                .unwrap()
                .write_all(b"continued-after-pause\n")
                .unwrap();
            assert!(!s(&term.call(json!({ "wait" : "PAUSED" }))["screen"])
                .contains("continued-after-pause"));
            term.call(json!({ "send" : " ", "wait" : "continued-after-pause" }));
            a.cli(&["plugin", "stop", "term"]);
            term.call(json!({ "wait" : "DISCONNECTED", "timeout_ms" : 15000 }));
            term.finish("q");
            a.cli(&["plugin", "start", "term"]);
            a.cli(&["tui", "term", "attach", "--snapshot"]);
        },
    );
}
