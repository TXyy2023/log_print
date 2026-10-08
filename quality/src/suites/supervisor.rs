use crate::{owned::Owned, suites::Suite, support::*};
use serde_json::{json, Value};
use std::{fs, path::Path};
pub fn file_plugin(path: impl AsRef<Path>, autostart: bool) -> Value {
    json!(
        { "id" : "source", "role" : "input", "bin" : "input-file", "autostart" :
        autostart, "streams" : [{ "id" : "source-log", "description" : "fixture bytes"
        }], "config" : { "path" : path.as_ref(), "mode" : "static", "from_start" : true,
        "chunk_bytes" : 4 } }
    )
}
fn stopped(app: &App) {
    until(|| app.inspect("status", None)["plugin_processes"][0]["state"] == "stopped");
}
pub fn run(t: &mut Suite) {
    t.case("starts_empty_and_stays_running_until_manual_stop", || {
        let a = App::configured(json!({ "plugins" : [] }));
        assert_eq!(a.inspect("streams", None), json!([]));
        pause(150);
        assert_eq!(a.inspect("streams", None), json!([]));
        assert!(a.inspect("status", None).get("supervisor").is_some());
        for args in [
            vec!["config", "set", "x", "--json", "{}"],
            vec!["session", "list", "x"],
        ] {
            assert!(!a.raw(&args).status.success())
        }
        a.stop();
    });
    t.case(
        "late_start_returns_stream_id_snapshot_survives_file_edit_and_restart",
        || {
            let td = tempfile::tempdir().unwrap();
            let source = td.path().join("source.log");
            let replacement = td.path().join("replacement.log");
            fs::write(&source, b"original\n").unwrap();
            fs::write(&replacement, b"wrong\n").unwrap();
            let mut config = json!({ "plugins" : [file_plugin(& source, false)] });
            let a = App::configured(config.clone());
            config["plugins"][0]["config"]["path"] = json!(replacement);
            write_json(&a.config, &config);
            let started = a.cli(&["plugin", "start", "source"]);
            let stream = s(&a.inspect("streams", None)[0]["id"]);
            assert!(started.contains(&stream));
            stopped(&a);
            assert_eq!(
                payload(&a.inspect("read", Some(&stream))["records"], None),
                b"original\n"
            );
            assert_eq!(
                a.inspect("stream", Some(&stream))["description"],
                "fixture bytes"
            );
            assert_eq!(
                a.inspect("config", None)["config"]["plugins"][0]["config"]["path"],
                json!(source)
            );
            a.cli(&["plugin", "restart", "source"]);
            until(|| arr(&a.inspect("read", Some(&stream))["records"]).len() >= 6);
            assert_eq!(
                payload(&a.inspect("read", Some(&stream))["records"], None),
                b"original\noriginal\n"
            );
        },
    );
    t.case("runtime_output_binding_uses_real_stream_id", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("source");
        fs::write(&source, b"bound-by-UUID\n").unwrap();
        let a = App::configured(json!(
            { "plugins" : [file_plugin(source, false), { "id" : "screen", "role"
            : "output", "bin" : "output-raw", "autostart" : false, "config" : {}
            }] }
        ));
        a.cli(&["plugin", "start", "source"]);
        let id = s(&a.inspect("streams", None)[0]["id"]);
        assert!(a
            .cli(&["plugin", "start", "screen", "--stream", &id])
            .contains("streams:"));
        until(|| text(a.temp.path().join("state.json.stdout.log")).contains("bound-by-UUID"));
        let r = a.cli(&["plugin", "stop", "screen"]);
        assert!(r.contains("forced: no") && r.contains("success: yes"));
        assert!(!arr(&a.inspect("read", Some(&id))["records"]).is_empty());
    });
    t.case("start_failure_cleans_its_state", || {
        let a = App::new(json!({ "plugins" : [file_plugin("/definitely/not/a/log/file", true)] }));
        assert!(!a
            .raw(&["start", "--config", a.config.to_str().unwrap()])
            .status
            .success());
        until(|| !a.state.exists());
    });
    t.case("core_crash_reaps_spawned_source_and_descendant", || {
        for transport in ["tcp", "udp"] {
            let td = tempfile::tempdir().unwrap();
            let marker = td.path().join("pids.json");
            let a = App::configured(json!(
                { "core" : { "transport" : transport }, "plugins" : [{ "id" :
                "source", "role" : "input", "bin" : "input-program", "streams" :
                [{ "id" : "crash-source" }], "config" : { "command" : fixture(),
                "args" : ["fixture", "tree", marker] } }] }
            ));
            let pids = eventually(8, || read_json(&marker));
            let state = read_json(&a.state).unwrap();
            let snapshot = a.inspect("status", None);
            let mut ids = vec![
                state["core_pid"].as_u64().unwrap(),
                state["pid"].as_u64().unwrap(),
                snapshot["plugin_processes"][0]["pid"].as_u64().unwrap(),
            ];
            ids.extend(arr(&pids).iter().map(|v| v.as_u64().unwrap()));
            let observed: Vec<_> = ids.into_iter().map(|p| Owned::new(p as u32)).collect();
            assert!(observed.iter().all(Owned::alive));
            observed[0].kill();
            eventually(25, || (!a.state.exists()).then_some(()));
            until(|| observed.iter().all(|p| !p.alive()));
        }
    });
    t.case("old_storage_config_and_duplicate_identity_rejected", || {
        for config in [
            json!({ "core" : { "save" : { "enabled" : true } } }),
            json!({ "plugins" : [{ "id" : "bad", "bin" : "input-file" }] }),
            json!(
                { "plugins" : [{ "id" : "x", "role" : "input", "bin" : "unused" }, {
                "id" : "x", "role" : "input", "bin" : "unused" }] }
            ),
        ] {
            let a = App::new(config);
            assert!(!a
                .raw(&["start", "--config", a.config.to_str().unwrap()])
                .status
                .success());
            until(|| !a.state.exists());
        }
    });
    t.case(
        "second_start_does_not_stop_or_overwrite_existing_instance",
        || {
            let a = App::configured(json!({}));
            let before = bytes(&a.state);
            assert!(!a
                .raw(&["start", "--config", a.config.to_str().unwrap()])
                .status
                .success());
            assert_eq!(bytes(&a.state), before);
            assert!(a.inspect("status", None).get("supervisor").is_some());
        },
    );
    t.case("udp_configuration_and_actual_file_input", || {
        let td = tempfile::tempdir().unwrap();
        let p = td.path().join("source");
        fs::write(&p, b"udp").unwrap();
        let a = App::configured(json!(
            { "core" : { "transport" : "udp" }, "plugins" : [file_plugin(p,
            true)] }
        ));
        let id = s(&a.inspect("streams", None)[0]["id"]);
        until(|| payload(&a.inspect("read", Some(&id))["records"], None) == b"udp");
        assert_eq!(
            a.inspect("config", None)["config"]["core"]["transport"],
            "udp"
        );
    });
}
pub fn failures(t: &mut Suite) {
    for action in ["stop", "restart", "instance-stop", "terminal-restart"] {
        t.case(&format!("failure_reporting_{action}"), || {
            let td = tempfile::tempdir().unwrap();
            let marker = td.path().join("runs");
            let terminal = action == "terminal-restart";
            let mut a = App::configured(json!(
                { "plugins" : [{ "id" : "probe", "role" : "input", "bin" :
                fixture(), "args" : ["fixture", "failure-peer"], "streams" : [{
                "id" : "probe-log" }], "config" : { "marker" : marker,
                "terminal_once" : terminal } }] }
            ));
            a.allow_cleanup_failure = true;
            if terminal {
                stopped(&a)
            }
            let args = if action == "instance-stop" {
                vec!["stop"]
            } else {
                vec!["plugin", if terminal { "restart" } else { action }, "probe"]
            };
            let r = a.raw(&args);
            assert!(!r.status.success());
            let out = String::from_utf8_lossy(&r.stdout);
            assert_eq!(text(&marker).lines().count(), if terminal { 2 } else { 1 });
            if action == "stop" {
                assert!(
                    out.contains("success: no") && out.contains('7') && out.contains("forced: no")
                );
            }
            if action == "instance-stop" {
                until(|| !a.state.exists());
                let both = format!("{out}{}", String::from_utf8_lossy(&r.stderr));
                assert!(both.contains('7') || both.contains("failed"));
            } else {
                assert_eq!(
                    a.inspect("status", None)["plugin_processes"][0]["state"],
                    "stopped"
                );
                if terminal {
                    assert_eq!(a.inspect("status", None)["plugins"][0]["connected"], false);
                }
            }
        });
    }
}
