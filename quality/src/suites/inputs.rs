use crate::{owned::Owned, suites::Suite, support::*};
use serde_json::{json, Value};
use std::{fs, io::Write, process::Command};
pub fn spec(binary: &str, config: Value) -> Value {
    json!(
        { "id" : "source", "role" : "input", "bin" : binary, "streams" : [{ "id" :
        "source-alias", "description" : "input acceptance fixture" }], "config" : config
        }
    )
}
fn core(p: &Value) -> Core {
    Core::new("tcp", json!({}), json!([p]))
}
fn program(mode: &str, args: Value) -> Value {
    let mut all = vec![json!("fixture"), json!(mode)];
    all.extend(arr(&args).iter().cloned());
    spec(
        "input-program",
        json!({ "command" : fixture(), "args" : all, "chunk_bytes" : 3 }),
    )
}
pub fn run(t: &mut Suite) {
    t.case("static_binary", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("bytes.bin");
        let mut data: Vec<u8> = (0..256).cycle().take(256 * 101).map(|i| i as u8).collect();
        data.extend(b"without-newline");
        fs::write(&source, &data).unwrap();
        let p = spec(
            "input-file",
            json!({ "path" : source, "mode" : "static", "chunk_bytes" : 701 }),
        );
        let c = core(&p);
        let mut run = Plugin::new(&c, &p, &[]);
        let report = run.wait(true);
        assert_eq!(report["state"], "source_eof");
        assert_eq!(report["bytes_sent"], data.len());
        assert_eq!(report["downstream_complete"], false);
        assert_eq!(payload(&c.records("source"), None), data);
        for (i, r) in arr(&c.records("source")).iter().enumerate() {
            assert_eq!(r["source_seq"], i + 1);
            assert_eq!(r["stream"], c.stream("source"));
            assert!(r["channel"].is_null());
        }
        assert_eq!(arr(&c.streams()).len(), 1);
        pause(50);
        assert_eq!(payload(&c.records("source"), None), data);
    });
    t.case("static_empty", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("empty");
        fs::write(&source, b"").unwrap();
        let p = spec("input-file", json!({ "path" : source, "mode" : "static" }));
        let c = core(&p);
        let r = Plugin::new(&c, &p, &[]).wait(true);
        assert_eq!(r["bytes_sent"], 0);
        assert_eq!(r["chunks_sent"], 0);
        assert_eq!(c.streams()[0]["head"], 0);
        assert_eq!(c.records("source"), json!([]));
    });
    t.case("follow", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("tail.log");
        fs::write(&source, b"OLD history must be skipped").unwrap();
        let p = spec("input-file", json!({ "path" : source, "poll_ms" : 10 }));
        let c = core(&p);
        let mut run = Plugin::new(&c, &p, &[]);
        run.ready("following");
        assert!(payload(&c.records("source"), None).is_empty());
        fs::OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"append")
            .unwrap();
        until(|| payload(&c.records("source"), None) == b"append");
        for (value, expected) in [("x", "appendx"), ("y", "appendxy")] {
            fs::write(&source, value).unwrap();
            until(|| payload(&c.records("source"), None) == expected.as_bytes());
        }
        fs::rename(&source, td.path().join("tail.log.1")).unwrap();
        until(|| run.report()["state"] == "path_missing");
        fs::write(&source, b"new-inode").unwrap();
        until(|| payload(&c.records("source"), None) == b"appendxynew-inode");
        run.stop();
        let rows = c.records("source");
        assert_eq!(arr(&c.streams()).len(), 1);
        for (i, r) in arr(&rows).iter().enumerate() {
            assert_eq!(r["source_seq"], i + 1)
        }
        let keys: std::collections::HashSet<_> = arr(&rows)
            .iter()
            .map(|r| s(&r["key"]).split(':').nth(1).unwrap().to_owned())
            .collect();
        assert_eq!(keys.len(), 4);
    });
    t.case("from_start", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("existing");
        fs::write(&source, b"initial bytes").unwrap();
        let p = spec(
            "input-file",
            json!({ "path" : source, "from_start" : true, "poll_ms" : 10 }),
        );
        let c = core(&p);
        let mut run = Plugin::new(&c, &p, &[]);
        until(|| payload(&c.records("source"), None) == b"initial bytes");
        run.stop();
    });
    t.case("program_channels", || {
        let p = program("channels", json!([]));
        let c = core(&p);
        let report = Plugin::new(&c, &p, &[]).wait(true);
        assert_eq!(report["state"], "source_exited");
        assert_eq!(report["exit_code"], 0);
        let rows = c.records("source");
        assert_eq!(payload(&rows, Some("stdout")), b"A\0\xffwithout-newline");
        assert_eq!(payload(&rows, Some("stderr")), b"err\0\xfe");
        assert_eq!(arr(&c.streams()).len(), 1);
        for (i, r) in arr(&rows).iter().enumerate() {
            assert_eq!(r["seq"], i + 1)
        }
        for channel in ["stdout", "stderr"] {
            for (i, r) in arr(&rows)
                .iter()
                .filter(|r| r["channel"] == channel)
                .enumerate()
            {
                assert_eq!(r["source_seq"], i + 1);
                assert!(s(&r["key"]).contains(&format!(":{channel}:")));
            }
        }
    });
    t.case("nonzero", || {
        let p = program("nonzero", json!([]));
        let c = core(&p);
        let report = Plugin::new(&c, &p, &[]).wait(false);
        assert_eq!(report["state"], "failed");
        assert!(s(&report["error"]).contains('7'));
        assert_eq!(
            payload(&c.records("source"), Some("stderr")),
            b"failure detail"
        );
    });
    t.case("stop_tree", || {
        let td = tempfile::tempdir().unwrap();
        let marker = td.path().join("pids");
        let p = program("tree", json!([marker]));
        let c = core(&p);
        let mut run = Plugin::new(&c, &p, &[]);
        run.ready("capturing");
        let ids = eventually(8, || read_json(&marker));
        let owned: Vec<_> = arr(&ids)
            .iter()
            .map(|v| Owned::new(v.as_u64().unwrap() as u32))
            .collect();
        assert!(owned.iter().all(Owned::alive));
        until(|| payload(&c.records("source"), Some("stdout")) == b"parent ready");
        run.stop();
        until(|| owned.iter().all(|p| !p.alive()));
    });
    t.case("missing_program", || {
        let td = tempfile::tempdir().unwrap();
        let p = spec(
            "input-program",
            json!({ "command" : td.path().join("does-not-exist") }),
        );
        let c = core(&p);
        let r = Plugin::new(&c, &p, &[]).wait(false);
        assert_eq!(r["state"], "failed");
        assert!(s(&r["error"]).contains("spawn"));
    });
    t.case("invalid_file", || {
        let td = tempfile::tempdir().unwrap();
        let paths = vec![td.path().join("missing"), td.path().to_owned()];
        #[cfg(unix)]
        let paths = {
            let mut paths = paths;
            let fifo = td.path().join("fifo");
            let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
            paths.push(fifo);
            paths
        };
        for path in paths {
            let p = spec("input-file", json!({ "path" : path, "mode" : "static" }));
            let c = core(&p);
            Plugin::new(&c, &p, &[]).wait(false);
            assert_eq!(c.records("source"), json!([]));
        }
    });
    t.case("core_loss", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("follow");
        fs::write(&source, b"old").unwrap();
        for p in [
            spec("input-file", json!({ "path" : source })),
            program("sleep", json!([])),
        ] {
            let c = core(&p);
            let mut run = Plugin::new(&c, &p, &[]);
            run.ready(if p["bin"] == "input-file" {
                "following"
            } else {
                "capturing"
            });
            let pid = c.process.as_ref().unwrap().0.id();
            Owned::new(pid).kill();
            assert_ne!(run.process.wait(8), 0);
            let log = text(&run.stderr);
            assert!(log.contains("Core") || log.contains("connection"));
        }
    });
    if cfg!(unix) && Command::new("tmux").arg("-V").output().is_ok() {
        t.case("tmux_live", tmux_live)
    } else {
        t.skip("tmux_live", "tmux is unavailable on this platform")
    }
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn tmux_live() {
    let td = tempfile::tempdir().unwrap();
    let short = tempfile::Builder::new()
        .prefix("lp-tmux-")
        .tempdir_in("/tmp")
        .unwrap();
    let server = short.path().join("server.sock");
    struct Server(std::path::PathBuf);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(&self.0)
                .arg("kill-server")
                .output();
        }
    }
    let _server = Server(server.clone());
    let tmux = |args: &[&str]| {
        let r = capture(Command::new("tmux").arg("-S").arg(&server).args(args), 6);
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        String::from_utf8(r.stdout).unwrap().trim().to_owned()
    };
    let control = td.path().join("control");
    let cmd = format!(
        "{} fixture tmux-source {}",
        quote(&fixture()),
        quote(control.to_str().unwrap())
    );
    let pane = tmux(&[
        "new-session",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-s",
        "acceptance",
        &cmd,
    ]);
    let pid: u32 = tmux(&["display-message", "-p", "-t", &pane, "#{pane_pid}"])
        .parse()
        .unwrap();
    let owned = Owned::new(pid);
    until(|| tmux(&["capture-pane", "-p", "-t", &pane]).contains("BEFORE_ATTACH"));
    let copy = td.path().join("input program's copy");
    fs::copy(bin("input-program"), &copy).unwrap();
    let p = spec(
        copy.to_str().unwrap(),
        json!({ "mode" : "tmux", "tmux_target" : pane, "tmux_socket" : server }),
    );
    {
        let c = core(&p);
        let mut run = Plugin::new(&c, &p, &[]);
        assert_eq!(run.ready("capturing")["history_imported"], false);
        assert!(payload(&c.records("source"), None).is_empty());
        fs::write(&control, "AFTER").unwrap();
        until(|| {
            String::from_utf8_lossy(&payload(&c.records("source"), Some("terminal")))
                .contains("TMUX_AFTER")
        });
        assert!(
            !String::from_utf8_lossy(&payload(&c.records("source"), None))
                .contains("BEFORE_ATTACH")
        );
        run.stop();
        until(|| tmux(&["display-message", "-p", "-t", &pane, "#{pane_pipe}"]) == "0");
        assert!(owned.alive());
        fs::write(&control, "STILL_RUNNING").unwrap();
        until(|| tmux(&["capture-pane", "-p", "-t", &pane]).contains("TMUX_STILL_RUNNING"));
        assert!(
            !String::from_utf8_lossy(&payload(&c.records("source"), None))
                .contains("STILL_RUNNING")
        );
    }
    for (kind, after) in [("existing", false), ("later", true)] {
        let path = td.path().join(kind);
        let command = format!("cat > {}", quote(path.to_str().unwrap()));
        if !after {
            tmux(&["pipe-pane", "-O", "-t", &pane, &command]);
        }
        let c = core(&p);
        let mut run = Plugin::new(&c, &p, &[]);
        if after {
            run.ready("capturing");
            tmux(&["pipe-pane", "-O", "-t", &pane, &command]);
            run.wait(true);
        } else {
            assert!(s(&run.wait(false)["error"]).contains("already has pipe-pane"));
        }
        assert_eq!(
            tmux(&["display-message", "-p", "-t", &pane, "#{pane_pipe}"]),
            "1"
        );
        fs::write(&control, kind).unwrap();
        until(|| text(&path).contains(&format!("TMUX_{kind}")));
        assert!(owned.alive());
        tmux(&["pipe-pane", "-t", &pane]);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let raced = td.path().join("raced");
        let wrapper_dir = td.path().join("bin");
        fs::create_dir(&wrapper_dir).unwrap();
        let real = String::from_utf8(capture(Command::new("which").arg("tmux"), 5).stdout).unwrap();
        let wrapper = wrapper_dir.join("tmux");
        let code = format!(
            "#!/bin/sh\nout=$({} \"$@\"); rc=$?\ncase \"$*\" in *display-message*pane_id*) {} -S {} pipe-pane -O -t {} {} ;; esac\nprintf '%s\\n' \"$out\"\nexit \"$rc\"\n",
            quote(real.trim()), quote(real.trim()), quote(server.to_str().unwrap()),
            quote(& pane), quote(& format!("cat > {}", quote(raced.to_str().unwrap())))
        );
        fs::write(&wrapper, code).unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        let path = format!(
            "{}:{}",
            wrapper_dir.display(),
            std::env::var("PATH").unwrap()
        );
        let c = core(&p);
        let mut run = Plugin::new(&c, &p, &[("PATH", &path)]);
        let r = run.wait(false);
        assert!(s(&r["error"]).contains("concurrent replacement"), "{r}");
        assert_eq!(
            tmux(&["display-message", "-p", "-t", &pane, "#{pane_pipe}"]),
            "1"
        );
        fs::write(&control, "RACED_PIPE").unwrap();
        until(|| text(&raced).contains("TMUX_RACED_PIPE"));
    }
}
