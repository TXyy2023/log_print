use crate::{suites::Suite, support::*};
use serde_json::{json, Value};
use std::{fs, process::Command};
fn readable(s: String) -> String {
    assert!(!s.trim().is_empty());
    assert!(serde_json::from_str::<Value>(&s).is_err());
    s
}
pub fn run(t: &mut Suite) {
    t.case("follow_file_lifecycle_and_named_controls", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("中文 file=a.log");
        fs::write(&source, "已有内容\n").unwrap();
        let a = App::native(&[
            "--input-file",
            &format!("source={}", source.display()),
            "--set",
            "source.from_start=true",
            "--output-raw",
            "screen",
            "--no-autostart",
            "screen",
            "--describe",
            "source=应用日志",
        ]);
        let id = s(&a.inspect("streams", None)[0]["id"]);
        assert!(readable(a.cli(&["streams"])).contains(&id));
        fs::write(&source, "已有内容\n新增内容\n").unwrap();
        until(|| {
            payload(&a.inspect("read", Some(&id))["records"], None)
                .windows("新增".len())
                .any(|x| x == "新增".as_bytes())
        });
        let data = readable(a.cli(&["read", &id]));
        for expected in ["已有内容", "新增内容", "source_seq:"] {
            assert!(data.contains(expected));
        }
        assert!(readable(a.cli(&["config"])).contains("command line"));
        assert!(readable(a.cli(&["config", "--plugin", "source"])).contains("from_start: yes"));
        assert!(readable(a.cli(&["resolve", "source"])).contains(&id));
        readable(a.cli(&["describe", &id, "编译输出"]));
        assert_eq!(a.inspect("stream", Some(&id))["description"], "编译输出");
        readable(a.cli(&[
            "call",
            "stream.describe",
            &format!("stream={id}"),
            "--text",
            "description=true",
        ]));
        assert_eq!(a.inspect("stream", Some(&id))["description"], "true");
        readable(a.cli(&["plugin", "call", "source", "config.get"]));
        readable(a.cli(&["plugin", "start", "screen", "--stream", &id]));
        assert!(readable(a.cli(&["plugin", "stop", "screen"])).contains("success: yes"));
        assert!(readable(a.cli(&["plugin", "restart", "screen"])).contains("started:"));
        assert!(readable(a.cli(&["status"])).contains("plugin_processes:"));
        let r = readable(a.cli(&["stop"]));
        assert!(r.contains("success: yes") && r.contains("forced: no"));
        assert!(!a.state.exists());
    });
    t.case("spawn_args_binary_payload_and_raw_bytes", || {
        let a = App::native(&[
            "--input-program",
            &format!("source={}", fixture()),
            "--list-text",
            "source.args=fixture",
            "--list-text",
            "source.args=binary",
            "--list-text",
            "source.args=001",
            "--text",
            "source.env.FLAG=false",
        ]);
        let id = s(&a.inspect("streams", None)[0]["id"]);
        until(|| {
            let rows = a.inspect("read", Some(&id));
            arr(&rows["records"])
                .iter()
                .any(|r| r["channel"] == "stdout")
                && arr(&rows["records"])
                    .iter()
                    .any(|r| r["channel"] == "stderr")
        });
        let expected = payload(&a.inspect("read", Some(&id))["records"], None);
        let result = a.raw(&["read", &id, "--raw"]);
        assert!(result.status.success());
        assert_eq!(result.stdout, expected);
        let rendered = readable(a.cli(&["read", &id]));
        assert!(
            rendered.contains("payload (hex): 6f 75 74 00 ff")
                && rendered.contains("payload: false")
        );
    });
    t.case("transform_and_both_archive_shortcuts", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("source.log");
        let archive = td.path().join("capture.log");
        let database = td.path().join("capture.sqlite");
        fs::write(&source, b"fixture archive\n").unwrap();
        let a = App::native(&[
            "--input-file",
            &format!("source={}", source.display()),
            "--set",
            "source.mode=static",
            "--no-autostart",
            "source",
            "--output-transform",
            "derived",
            "--set",
            "derived.number=true",
            "--output-file",
            &format!("archive={}", archive.display()),
            "--read",
            "archive=derived",
            "--output-sqlite",
            &format!("database={}", database.display()),
            "--read",
            "database=derived",
        ]);
        readable(a.cli(&["plugin", "start", "source"]));
        let expected = b"[n=1] fixture archive\n";
        until(|| bytes(&archive) == expected);
        assert!(
            readable(a.cli(&["plugin", "call", "archive", "status.get"])).contains("confirmed:")
        );
        a.stop();
        assert_eq!(bytes(&archive), expected);
        let db = rusqlite::Connection::open(database).unwrap();
        let rows: Vec<Vec<u8>> = db
            .prepare("SELECT payload FROM records")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows.concat(), expected);
    });
    t.case("multi_input_explicit_binding_and_empty_instance", || {
        let a = App::native(&[]);
        assert_eq!(a.cli(&["streams"]), "No streams.\n");
        readable(a.cli(&["status"]));
        a.stop();
        let td = tempfile::tempdir().unwrap();
        let first = td.path().join("first");
        let second = td.path().join("second");
        fs::write(&first, b"first").unwrap();
        fs::write(&second, b"second").unwrap();
        let a = App::native(&[
            "--input-file",
            &format!("first={}", first.display()),
            "--input-file",
            &format!("second={}", second.display()),
            "--set",
            "first.mode=static",
            "--set",
            "second.mode=static",
            "--output-raw",
            "screen",
            "--read",
            "screen=second",
        ]);
        let streams = a.inspect("streams", None);
        assert_eq!(arr(&streams).len(), 2);
        for owner in ["first", "second"] {
            assert!(arr(&streams).iter().any(|r| r["owner"] == owner));
        }
        assert!(readable(a.cli(&["streams"])).contains("first"));
        let path = a.temp.path().join("state.json.stdout.log");
        until(|| text(&path).contains("second"));
        assert!(!text(&path).contains("first"));
    });
    t.case("foreground_launch_stops_owned_children", || {
        let a = App::new(json!({}));
        let out = fs::File::create(a.temp.path().join("stdout")).unwrap();
        let err = fs::File::create(a.temp.path().join("stderr")).unwrap();
        let mut child = Process::spawn(
            Command::new(bin("log-print"))
                .arg("--state")
                .arg(&a.state)
                .arg("run")
                .stdout(out)
                .stderr(err),
        );
        until(|| read_json(&a.state).is_some_and(|v| v.get("address").is_some()));
        a.stop();
        assert_eq!(child.wait(5), 0);
    });
}
