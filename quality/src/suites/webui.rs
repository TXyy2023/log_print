use crate::{suites::Suite, support::*};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader},
    sync::OnceLock,
    time::Duration,
};
pub fn client() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap()
    })
}
pub fn response(
    url: &str,
    method: Option<&str>,
    args: Value,
    origin: bool,
) -> reqwest::blocking::Response {
    let req = if let Some(m) = method {
        client()
            .post(format!("{url}/api/control"))
            .json(&json!({ "method" : m, "args" : args }))
    } else {
        client().get(format!("{url}/api/state"))
    };
    let req = if origin {
        req.header("Origin", url)
    } else {
        req
    };
    req.send().unwrap()
}
pub fn request(url: &str, method: &str, args: Value) -> Value {
    let r = response(url, (!method.is_empty()).then_some(method), args, true);
    let status = r.status();
    let body = r.text().unwrap();
    assert!(status.is_success(), "HTTP {status}: {body}");
    serde_json::from_str(&body).unwrap()
}
pub fn job(url: &str, method: &str, args: Value) -> String {
    s(&request(url, method, args)["query"])
}
pub fn complete(url: &str, id: &str, timeout: u64) -> Value {
    eventually(timeout, || {
        let v = request(url, "query.get", json!({ "query" : id }));
        assert_ne!(v["status"]["state"], "failed", "{v}");
        (v["status"]["state"] != "running").then_some(v)
    })
}
pub fn rows(url: &str, id: &str, timeout: u64) -> (Vec<Value>, Value) {
    let value = complete(url, id, timeout);
    let mut rows = vec![];
    let mut offset = 0;
    while offset < value["total"].as_u64().unwrap() {
        let page = request(url, "query.get", json!({ "query" : id, "offset" : offset }));
        let next = page["next"].as_u64().unwrap();
        assert!(next > offset);
        rows.extend(arr(&page["rows"]).iter().cloned());
        offset = next;
    }
    (rows, value)
}
fn base(kind: &str, config: Value, archive: Option<Value>) -> Value {
    let mut plugins = vec![
        super::outputs::source(),
        json!({ "id" : "derive", "role" : "output", "bin" :
        "unused", "reads" : ["raw"] }),
        json!({ "id" : "web", "role" : "output", "bin" :
        format!("output-{kind}"), "read_all" : true, "config" : config }),
    ];
    if let Some(p) = archive {
        plugins.push(p)
    }
    json!(plugins)
}
fn archive(path: &std::path::Path) -> Value {
    json!(
        { "id" : "archive", "role" : "output", "bin" : "output-file", "read_all" : true,
        "config" : { "mode" : "create", "streams" : [], "discover_streams" : true,
        "sqlite" : { "path" : path }, "fail_on_gap" : false, "commit" : { "max_records" :
        1, "max_delay_ms" : 10 } } }
    )
}
fn texts(rows: &[Value]) -> Vec<String> {
    rows.iter().map(|r| s(&r["text"])).collect()
}
fn checkpoint(db: &Connection, stream: &str, next: u64) -> bool {
    db.query_row(
        "SELECT next FROM checkpoints WHERE stream=?",
        [stream],
        |r| r.get::<_, String>(0),
    )
    .ok()
        == Some(next.to_string())
}
fn sse(url: &str) -> Value {
    let response = client()
        .get(format!("{url}/api/events"))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    let mut reader = BufReader::new(response);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("event: state"));
    line.clear();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(line.trim().strip_prefix("data: ").unwrap()).unwrap()
}
pub fn run(t: &mut Suite, kind: &str) {
    t.case("memory_only_http_sse_revision_and_empty_streams", || {
        let td = tempfile::tempdir().unwrap();
        let plugins = base(
            kind,
            json!({ "state_path" : td.path().join("pages.sqlite3") }),
            None,
        );
        let c = Core::new("tcp", json!({ "buffer_records" : 2 }), plugins.clone());
        let mut web = Plugin::new(&c, &plugins[2], &[]);
        let url = s(&web.ready("serving")["url"]);
        until(|| arr(&request(&url, "", json!({}))["streams"]).len() == 1);
        let html = client().get(&url).send().unwrap().text().unwrap();
        assert!(html.contains(if kind == "webui" {
            "/assets/"
        } else {
            "output-tui: attach"
        }));
        assert!(!html.contains("cdn"));
        assert!(sse(&url).get("pages").is_some());
        assert_eq!(
            response(
                &url,
                Some("page.create"),
                json!({ "name" : "blocked" }),
                false
            )
            .status(),
            403
        );
        let rev = request(&url, "", json!({}))["revision"].clone();
        request(
            &url,
            "page.create",
            json!({ "name" : "saved", "revision" : rev }),
        );
        assert_eq!(
            response(
                &url,
                Some("page.set"),
                json!({ "title" : "lost", "revision" :
                rev }),
                true
            )
            .status(),
            409
        );
        let mut w = c.rpc("source");
        let stream = c.stream("source");
        w.call("stream.claim", json!({ "stream" : stream }));
        for i in 1..15 {
            w.publish(&stream, format!("value={i}\n").as_bytes(), json!({}));
            pause(5);
        }
        let (r, v) = rows(&url, &job(&url, "history.read", json!({})), 15);
        assert_eq!(v["status"]["coverage"]["mode"], "memory_only");
        assert!(v["status"]["coverage"]["archive_id"].is_null());
        assert!(!r.is_empty());
        assert_eq!(
            v["status"]["coverage"]["streams"][0]["history_available"],
            false
        );
        web.stop();
    });
    t.case(
        "archive_overflow_dynamic_streams_search_context_curve_fixed_boundary",
        || {
            let td = tempfile::tempdir().unwrap();
            let path = td.path().join("history.sqlite");
            let arch = archive(&path);
            let plugins = base(
                kind,
                json!(
                    { "state_path" : td.path().join("pages.sqlite3"), "history_path" :
                    path, "history_plugin" : "archive" }
                ),
                Some(arch.clone()),
            );
            let c = Core::new("tcp", json!({ "buffer_records" : 4 }), plugins.clone());
            let mut file = Plugin::new(&c, &arch, &[]);
            let mut web = Plugin::new(&c, &plugins[2], &[]);
            file.ready("archiving");
            let url = s(&web.ready("serving")["url"]);
            let mut w = c.rpc("source");
            let stream = c.stream("source");
            w.call("stream.claim", json!({ "stream" : stream }));
            for i in 1..421 {
                w.publish(
                    &stream,
                    format!("value={i}\n").as_bytes(),
                    json!({ "channel" : "stdout" }),
                );
                pause(3);
            }
            let db = Connection::open(&path).unwrap();
            until(|| checkpoint(&db, &stream, 421));
            assert!(
                c.admin().call("stream.get", json!({ "stream" : stream }))["oldest"]
                    .as_u64()
                    .unwrap()
                    > 100
            );
            let query = job(&url, "history.read", json!({ "streams" : [stream] }));
            complete(&url, &query, 15);
            for i in 421..431 {
                w.publish(
                    &stream,
                    format!("value={i}\n").as_bytes(),
                    json!({ "channel" : "stdout" }),
                );
                pause(3);
            }
            let (r, _) = rows(&url, &query, 15);
            assert_eq!(
                texts(&r),
                (1..421).map(|i| format!("value={i}")).collect::<Vec<_>>()
            );
            let identities: std::collections::HashSet<_> = r
                .iter()
                .map(|r| {
                    format!(
                        "{}:{}:{}:{}",
                        r["stream"], r["epoch"], r["seq"], r["offset"]
                    )
                })
                .collect();
            assert_eq!(identities.len(), r.len());
            let (early, _) = rows(
                &url,
                &job(
                    &url,
                    "history.search",
                    json!({ "streams" : [stream], "regex" : "^value=1$" }),
                ),
                15,
            );
            assert_eq!(early[0]["seq"], "1");
            let ns = s(&early[0]["observed_ts_ns"]).parse::<u64>().unwrap();
            let (exact, _) = rows(
                &url,
                &job(
                    &url,
                    "history.search",
                    json!(
                        { "streams" : [stream], "regex" : "^value=1$", "time_from" : ns
                        .to_string(), "time_end" : ns.to_string() }
                    ),
                ),
                15,
            );
            assert_eq!(exact.len(), 1);
            let (outside, _) = rows(
                &url,
                &job(
                    &url,
                    "history.search",
                    json!(
                        { "streams" : [stream], "regex" : "^value=1$", "time_from" : (ns
                        + 1).to_string() }
                    ),
                ),
                15,
            );
            assert!(outside.is_empty());
            let (context, _) = rows(
                &url,
                &job(
                    &url,
                    "history.context",
                    json!({ "streams" : [stream], "seq" : 2, "before" : 1, "after" : 1 }),
                ),
                15,
            );
            assert_eq!(texts(&context), ["value=1", "value=2", "value=3"]);
            let (curve, _) = rows(
                &url,
                &job(
                    &url,
                    "history.curve",
                    json!({ "streams" : [stream], "regex" : r"value=(?P<value>\d+)" }),
                ),
                15,
            );
            assert_eq!(curve[0]["value"], 1.0);
            assert_eq!(curve.last().unwrap()["value"], 430.0);
            assert!(curve.len() <= 2000);
            {
                let mut derived = c.rpc("derive");
                let new = s(&derived.call(
                    "stream.create",
                    json!({ "description" : "dynamic", "parents" : [stream] }),
                )["id"]);
                until(|| {
                    db.query_row("SELECT 1 FROM streams WHERE stream=?", [&new], |r| {
                        r.get::<_, i64>(0)
                    })
                    .is_ok()
                });
                derived.publish(
                    &new,
                    b"value=999\n",
                    json!({ "upstream" : { & stream : 430 } }),
                );
                until(|| {
                    db.query_row("SELECT COUNT(*) FROM records WHERE stream=?", [&new], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap()
                        == 1
                });
                until(|| {
                    arr(&request(&url, "", json!({}))["streams"])
                        .iter()
                        .any(|v| v["id"] == new)
                });
                let panel = request(
                    &url,
                    "panel.add",
                    json!({ "kind" : "log", "streams" : [new] }),
                )["result"]["id"]
                    .clone();
                until(|| {
                    !arr(&request(&url, "panel.data", json!({ "panel" : panel }))["rows"])
                        .is_empty()
                });
                assert_eq!(
                    texts(arr(&request(
                        &url,
                        "panel.data",
                        json!({ "panel" : panel })
                    )["rows"])),
                    ["value=999"]
                );
            }
            for (b, ch) in [
                (b"val".as_slice(), "stdout"),
                (b"err\n", "stderr"),
                (b"ue=777\n", "stdout"),
            ] {
                w.publish(&stream, b, json!({ "channel" : ch }));
            }
            pause(150);
            let (split, _) = rows(
                &url,
                &job(
                    &url,
                    "history.search",
                    json!({ "streams" : [stream], "text" : "777" }),
                ),
                15,
            );
            assert_eq!(split[0]["text"], "value=777");
            assert_ne!(split[0]["seq"], split[0]["end_seq"]);
            file.stop();
            until(|| request(&url, "", json!({}))["archive_writer"]["connected"] == false);
            let stopped = complete(
                &url,
                &job(&url, "history.read", json!({ "streams" : [stream] })),
                15,
            );
            assert_eq!(stopped["status"]["coverage"]["writer"]["connected"], false);
            assert_eq!(
                stopped["status"]["coverage"]["streams"][0]["history_available"],
                true
            );
            assert!(response(
                &url,
                Some("history.read"),
                json!({ "streams" : [stream],
                "epoch" : "old-runtime" }),
                true
            )
            .status()
            .is_client_error());
            db.execute(
                "UPDATE metadata SET value=? WHERE key='archive_id'",
                [uuid::Uuid::new_v4().to_string()],
            )
            .unwrap();
            assert!(response(
                &url,
                Some("history.read"),
                json!({ "streams" : [stream] }),
                true
            )
            .status()
            .is_client_error());
            web.stop();
        },
    );
    t.case("late_archive_write_failure_and_coverage", || {
        let td = tempfile::tempdir().unwrap();
        let path = td.path().join("late.sqlite");
        let arch = archive(&path);
        let plugins = base(
            kind,
            json!(
                { "state_path" : td.path().join("pages.sqlite3"), "history_path" :
                path, "history_plugin" : "archive" }
            ),
            Some(arch.clone()),
        );
        let c = Core::new("tcp", json!({ "buffer_records" : 2 }), plugins.clone());
        let mut web = Plugin::new(&c, &plugins[2], &[]);
        let url = s(&web.ready("serving")["url"]);
        let mut w = c.rpc("source");
        let stream = c.stream("source");
        w.call("stream.claim", json!({ "stream" : stream }));
        for i in 1..7 {
            w.publish(&stream, format!("early={i}\n").as_bytes(), json!({}));
            pause(10);
        }
        let before = complete(
            &url,
            &job(&url, "history.read", json!({ "streams" : [stream] })),
            15,
        );
        assert!(!before["status"]["coverage"]["archive_error"].is_null());
        let mut file = Plugin::new(
            &c,
            &arch,
            &[
                ("LOG_PRINT_ARCHIVE_TESTING", "1"),
                ("LOG_PRINT_ARCHIVE_ERRORPOINT", "sqlite_commit"),
                ("LOG_PRINT_ARCHIVE_TEST_DELAY_MS", "100"),
            ],
        );
        file.ready("archiving");
        w.publish(&stream, b"after archive\n", json!({}));
        assert_ne!(file.process.wait(8), 0);
        until(|| request(&url, "", json!({}))["archive_writer"]["report"]["state"] == "failed");
        let failed = complete(
            &url,
            &job(&url, "history.read", json!({ "streams" : [stream] })),
            15,
        );
        let coverage = &failed["status"]["coverage"];
        assert_eq!(coverage["writer"]["report"]["state"], "failed");
        assert!(s(&coverage["writer"]["report"]["error"]).contains("commit"));
        assert_eq!(coverage["streams"][0]["archived"]["first"], "5");
        assert_eq!(coverage["streams"][0]["archived"]["last"], "4");
        assert!(!coverage["streams"][0]["uncommitted"].is_null());
        web.stop();
    });
    t.case("udp_budget_and_byte_context", || {
        let td = tempfile::tempdir().unwrap();
        let plugins = base(
            kind,
            json!({ "state_path" : td.path().join("pages.sqlite3") }),
            None,
        );
        let c = Core::new("udp", json!({ "buffer_records" : 64 }), plugins.clone());
        let mut web = Plugin::new(&c, &plugins[2], &[]);
        let url = s(&web.ready("serving")["url"]);
        let mut w = c.rpc("source");
        let stream = c.stream("source");
        w.call("stream.claim", json!({ "stream" : stream }));
        w.publish(&stream, b"zero\none\ntwo\nthree\nfour\n", json!({}));
        pause(50);
        let epoch = c.admin().call("stream.get", json!({ "stream" : stream }))["epoch"].clone();
        assert_eq!(
            arr(&c.admin().call(
                "read.range",
                json!({ "stream" : stream, "epoch" :
                epoch, "from" : 1, "end" : 1, "limit" : 64 })
            )["records"])
            .len(),
            1
        );
        let (context, _) = rows(
            &url,
            &job(
                &url,
                "history.context",
                json!(
                    { "streams" : [stream], "seq" : 1, "byte_offset" : 9, "before" :
                    1, "after" : 1 }
                ),
            ),
            15,
        );
        assert_eq!(texts(&context), ["one", "two", "three"]);
        let data = format!("{}\n", "x".repeat(300)).repeat(20);
        for _ in 0..10 {
            w.publish(&stream, data.as_bytes(), json!({}));
            pause(20);
        }
        until(|| request(&url, "", json!({}))["streams"][0]["head"] == 11);
        let id = job(&url, "history.read", json!({ "streams" : [stream] }));
        complete(&url, &id, 15);
        let reply = c.admin().call(
            "control",
            json!(
                { "target" : "web", "method" : "query.get", "args" : { "query" :
                id } }
            ),
        );
        assert!(reply.to_string().len() < 60 * 1024);
        assert!(reply["next"].as_u64().unwrap() < reply["total"].as_u64().unwrap());
        assert!(s(&c.admin().call(
            "control",
            json!({ "target" : "web", "method" : "url"
            })
        )["url"])
        .starts_with("http://"));
        web.stop();
    });
    t.case("background_cancel_and_concurrency", || {
        let td = tempfile::tempdir().unwrap();
        let plugins = base(
            kind,
            json!({ "state_path" : td.path().join("pages.sqlite3") }),
            None,
        );
        let c = Core::new("tcp", json!({ "buffer_records" : 64 }), plugins.clone());
        let mut web = Plugin::new(&c, &plugins[2], &[]);
        let url = s(&web.ready("serving")["url"]);
        let mut w = c.rpc("source");
        let stream = c.stream("source");
        w.call("stream.claim", json!({ "stream" : stream }));
        for _ in 0..20 {
            w.publish(&stream, &b"x\n".repeat(3000), json!({}));
        }
        let q1 = job(&url, "history.read", json!({ "streams" : [stream] }));
        let q2 = job(&url, "history.read", json!({ "streams" : [stream] }));
        assert_eq!(
            response(
                &url,
                Some("history.read"),
                json!({ "streams" : [stream] }),
                true
            )
            .status(),
            429
        );
        for q in [&q1, &q2] {
            request(&url, "query.cancel", json!({ "query" : q }));
        }
        for q in [&q1, &q2] {
            assert_eq!(complete(&url, q, 15)["status"]["state"], "cancelled");
        }
        web.stop();
    });
    t.case(
        "pause_is_a_shared_backend_frame_while_collection_continues",
        || {
            let td = tempfile::tempdir().unwrap();
            let plugins = base(
                kind,
                json!({ "state_path" : td.path().join("pages.sqlite3") }),
                None,
            );
            let c = Core::new("tcp", json!({ "buffer_records" : 2 }), plugins.clone());
            let mut web = Plugin::new(&c, &plugins[2], &[]);
            let url = s(&web.ready("serving")["url"]);
            let mut w = c.rpc("source");
            let stream = c.stream("source");
            w.call("stream.claim", json!({ "stream" : stream }));
            for i in 1..10 {
                w.publish(&stream, format!("value={i}\n").as_bytes(), json!({}));
                pause(10);
            }
            let panel =
                request(&url, "panel.add", json!({ "kind" : "log" }))["result"]["id"].clone();
            until(|| {
                !arr(&request(&url, "panel.data", json!({ "panel" : panel }))["rows"]).is_empty()
            });
            request(
                &url,
                "panel.set",
                json!({ "panel" : panel, "paused" : true }),
            );
            let frozen = request(&url, "panel.data", json!({ "panel" : panel }));
            assert_eq!(frozen["frozen"], true);
            for i in 10..31 {
                w.publish(&stream, format!("value={i}\n").as_bytes(), json!({}));
                pause(5);
            }
            assert_eq!(
                request(&url, "panel.data", json!({ "panel" : panel }))["rows"],
                frozen["rows"]
            );
            assert_eq!(
                c.admin().call(
                    "control",
                    json!({ "target" : "web", "method" :
                "panel.data", "args" : { "panel" : panel } })
                )["rows"],
                frozen["rows"]
            );
            assert_eq!(
                c.admin().call("stream.get", json!({ "stream" : stream }))["head"],
                30
            );
            request(
                &url,
                "panel.set",
                json!({ "panel" : panel, "paused" : false }),
            );
            until(|| {
                arr(&request(&url, "panel.data", json!({ "panel" : panel }))["rows"])
                    .last()
                    .is_some_and(|r| r["text"] == "value=30")
            });
            web.stop();
        },
    );
    t.case(
        "udp_catalog_is_complete_when_descriptions_exceed_one_frame",
        || {
            let td = tempfile::tempdir().unwrap();
            let mut plugins = base(
                kind,
                json!({ "state_path" : td.path().join("pages.sqlite3") }),
                None,
            );
            for i in 0..24 {
                plugins.as_array_mut().unwrap().push(json!(
                    { "id" : format!("extra-{i}"), "role" : "input", "bin" :
                    "unused", "streams" : [{ "id" : format!("flow-{i}"),
                    "description" : "x".repeat(4000) }] }
                ));
            }
            let c = Core::new("udp", json!({}), plugins.clone());
            let mut web = Plugin::new(&c, &plugins[2], &[]);
            let url = s(&web.ready("serving")["url"]);
            until(|| arr(&request(&url, "", json!({}))["streams"]).len() == 25);
            let first = c.admin().call(
                "control",
                json!(
                    { "target" : "web", "method" : "streams", "args" : { "offset" : 0
                    } }
                ),
            );
            assert!(first["next"].as_u64().unwrap() < first["total"].as_u64().unwrap());
            assert_eq!(
                c.admin().call(
                    "control",
                    json!({ "target" : "web", "method" : "streams",
                "args" : { "offset" : first["next"] } })
                )["next"],
                25
            );
            web.stop();
        },
    );
    t.case("slow_archive_keeps_gaps_and_later_context", || {
        let td = tempfile::tempdir().unwrap();
        let path = td.path().join("gaps.sqlite");
        let mut arch = archive(&path);
        arch["config"]["queue"] = json!({ "max_records" : 1 });
        let plugins = base(
            kind,
            json!(
                { "state_path" : td.path().join("pages.sqlite3"), "history_path" :
                path, "history_plugin" : "archive" }
            ),
            Some(arch.clone()),
        );
        let c = Core::new(
            "tcp",
            json!({ "buffer_records" : 1, "queue_records" : 1 }),
            plugins.clone(),
        );
        let mut file = Plugin::new(
            &c,
            &arch,
            &[
                ("LOG_PRINT_ARCHIVE_TESTING", "1"),
                ("LOG_PRINT_ARCHIVE_TEST_DELAY_MS", "50"),
            ],
        );
        file.ready("archiving");
        let stream = c.stream("source");
        {
            let mut w = c.rpc("source");
            w.call("stream.claim", json!({ "stream" : stream }));
            for i in 1..501 {
                w.publish(
                    &stream,
                    format!("{}\nvalue={i}\n", "p".repeat(48 * 1024)).as_bytes(),
                    json!({ "channel" : "stdout" }),
                );
            }
        }
        let db = Connection::open(&path).unwrap();
        eventually(55, || {
            assert!(file.process.exited().is_none(), "{}", text(&file.stderr));
            checkpoint(&db, &stream, 501).then_some(())
        });
        assert!(
            db.query_row("SELECT COUNT(*) FROM gaps", [], |r| r.get::<_, i64>(0))
                .unwrap()
                > 0
        );
        let mut web = Plugin::new(&c, &plugins[2], &[]);
        let url = s(&web.ready("serving")["url"]);
        let (r, v) = rows(
            &url,
            &job(&url, "history.read", json!({ "streams" : [stream] })),
            60,
        );
        assert!(v["status"]["coverage"]["gap_count"].as_u64().unwrap() > 0);
        assert_eq!(r.last().unwrap()["text"], "value=500");
        assert!(r.iter().any(|r| r["kind"] == "gap"));
        let (curve, _) = rows(
            &url,
            &job(
                &url,
                "history.curve",
                json!(
                    { "streams" : [stream], "channels" : ["stdout"], "text" :
                    "value=", "time_from" : "1", "regex" : r"value=(?P<value>\d+)" }
                ),
            ),
            60,
        );
        assert!(curve
            .iter()
            .any(|v| v["gap"] == true && v["value"].is_null()));
        assert_eq!(curve.last().unwrap()["value"], 500.0);
        web.stop();
        file.stop();
    });
    canvas(t, kind);
    restore(t, kind);
}
fn canvas(t: &mut Suite, kind: &str) {
    t.case("canvas_cli_atomic_layout_revision_and_restart", || {
        let a = App::native(&[&format!("--output-{kind}"), "web"]);
        let ctl = |m: &str, args: Value| a.control("web", m, args);
        let url = s(&ctl("url", json!({}))["url"]);
        let cli = |args: &[&str]| {
            let mut all = vec![kind, "web"];
            all.extend_from_slice(args);
            a.cli(&all)
        };
        cli(&[
            "page",
            "create",
            "--name",
            "canvas",
            "--layout-mode",
            "canvas",
            "--theme",
            "light",
        ]);
        cli(&["page", "select", "--page", "canvas"]);
        cli(&[
            "panel",
            "add",
            "--title",
            "Console",
            "--left",
            "-24.5",
            "--top",
            "32",
            "--panel-width",
            "760",
            "--panel-height",
            "480",
        ]);
        let first = s(&ctl("page.get", json!({}))["page"]["panels"][0]["id"]);
        cli(&["panel", "clone", "--panel", &first, "--title", "Second"]);
        let second = s(&ctl("page.get", json!({}))["page"]["panels"][1]["id"]);
        cli(&[
            "layout",
            "set",
            "--place",
            &format!("{first}=0,0,760,480"),
            "--place",
            &format!("{second}=800,0,520,360"),
        ]);
        cli(&[
            "panel",
            "set",
            "--panel",
            &second,
            "--hidden",
            "true",
            "--locked",
            "true",
            "--font-size",
            "14",
            "--row-height",
            "32",
            "--z-index",
            "2",
        ]);
        cli(&[
            "page",
            "set",
            "--view-x",
            "-80",
            "--view-y",
            "24",
            "--view-zoom",
            "0.75",
            "--show-minimap",
            "true",
            "--show-grid",
            "false",
            "--snap",
            "false",
            "--sidebar-open",
            "false",
            "--inspector-open",
            "true",
            "--active-panel",
            &first,
            "--tool",
            "pan",
        ]);
        let before = ctl("page.get", json!({}))["page"].clone();
        assert_eq!(before["layout_mode"], "canvas");
        assert_eq!(before["view_zoom"], 0.75);
        assert_eq!(before["panels"][1]["left"], 800.0);
        assert_eq!(before["panels"][1]["hidden"], true);
        let event = sse(&url);
        assert_eq!(
            arr(&event["pages"])
                .iter()
                .find(|p| p["id"] == event["selected"])
                .unwrap(),
            &before
        );
        let revision = request(&url, "", json!({}))["revision"].clone();
        assert!(!a
            .raw(&[
                kind,
                "web",
                "layout",
                "set",
                "--place",
                &format!("{first}=40,40,760,480"),
                "--place",
                "missing=0,0,500,300"
            ])
            .status
            .success());
        assert_eq!(ctl("page.get", json!({}))["page"], before);
        assert_eq!(request(&url, "", json!({}))["revision"], revision);
        cli(&["page", "set", "--view-zoom", "0.8"]);
        assert!(!a
            .raw(&[
                kind,
                "web",
                "page",
                "set",
                "--revision",
                &revision.to_string(),
                "--view-zoom",
                "1"
            ])
            .status
            .success());
        let expected = ctl("page.get", json!({}))["page"].clone();
        a.cli(&["plugin", "restart", "web"]);
        assert_eq!(ctl("page.get", json!({}))["page"], expected);
        a.stop();
        a.start();
        assert_eq!(ctl("page.get", json!({}))["page"], expected);
        cli(&["panel", "remove", "--panel", &first]);
        assert!(ctl("page.get", json!({}))["page"]["active_panel"].is_null());
    });
}
fn restore(t: &mut Suite, kind: &str) {
    t.case("cli_pages_restore_launch_modes_and_archive_flags", || {
        let td = tempfile::tempdir().unwrap();
        let source = td.path().join("app.log");
        fs::write(&source, "").unwrap();
        {
            let a = App::native(&[
                "--input-file",
                &format!("source={}", source.display()),
                &format!("--output-{kind}"),
                "web",
                &format!("--{kind}-archive"),
                &format!("web={}/capture", td.path().display()),
            ]);
            let ctl = |m: &str, args: Value| a.control("web", m, args);
            let cli = |args: &[&str]| {
                let mut all = vec![kind, "web"];
                all.extend_from_slice(args);
                a.cli(&all)
            };
            cli(&["page", "create", "--name", "dashboard", "--title", "监控"]);
            cli(&["page", "select", "--page", "dashboard"]);
            cli(&[
                "panel",
                "add",
                "--page",
                "dashboard",
                "--title",
                "Logs",
                "--kind",
                "log",
                "--h",
                "7",
                "--stream",
                "raw-waiting",
            ]);
            let panel =
                s(&ctl("page.get", json!({ "page" : "dashboard" }))["page"]["panels"][0]["id"]);
            cli(&[
                "panel",
                "set",
                "--panel",
                &panel,
                "--text",
                "saved-filter",
                "--paused",
                "true",
                "--format",
                "hex",
                "--x",
                "1",
                "--w",
                "11",
            ]);
            cli(&[
                "panel",
                "set",
                "--panel",
                &panel,
                "--column",
                "time",
                "--column",
                "seq",
                "--column",
                "text",
                "--column-width",
                "text=650",
                "--sort-column",
                "seq",
                "--sort-order",
                "desc",
            ]);
            let saved = ctl("panel.get", json!({ "panel" : panel }))["panel"].clone();
            assert_eq!(
                arr(&saved["column_state"])
                    .iter()
                    .find(|c| c["colId"] == "text")
                    .unwrap()["width"],
                650.0
            );
            assert_eq!(
                arr(&saved["column_state"])
                    .iter()
                    .find(|c| c["colId"] == "seq")
                    .unwrap()["sort"],
                "desc"
            );
            cli(&["page", "clone", "--page", "dashboard", "--name", "copy"]);
            cli(&["page", "delete", "--page", "copy"]);
            let before = ctl("page.get", json!({ "page" : "dashboard" }))["page"].clone();
            a.cli(&["plugin", "restart", "web"]);
            assert_eq!(
                ctl("page.get", json!({ "page" : "dashboard" }))["page"],
                before
            );
            a.stop();
            a.start();
            assert_eq!(
                ctl("page.get", json!({ "page" : "dashboard" }))["page"],
                before
            );
            let url = s(&ctl("url", json!({}))["url"]);
            assert_eq!(request(&url, "", json!({}))["selected"], before["id"]);
            let archives: Vec<_> = fs::read_dir(td.path().join("capture"))
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|s| s == "sqlite"))
                .collect();
            assert_eq!(archives.len(), 2);
            let db = Connection::open(&archives[0]).unwrap();
            assert_eq!(
                db.query_row(
                    "SELECT value FROM metadata WHERE key='schema_version'",
                    [],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "3"
            );
        }
        {
            let a = App::native(&[
                "--input-file",
                &format!("source={}", source.display()),
                "--output-sqlite",
                &format!("archive={}/explicit.sqlite", td.path().display()),
                &format!("--output-{kind}"),
                "web",
                &format!("--{kind}-history"),
                "web=archive",
            ]);
            assert!(a.inspect("status", None).to_string().contains("archive"));
            assert!(td.path().join("explicit.sqlite").exists());
        }
        let invalid = App::new(json!({}));
        assert!(!invalid
            .raw(&[
                "start",
                &format!("--output-{kind}"),
                "web",
                &format!("--{kind}-archive"),
                &format!("web={}/a", td.path().display()),
                &format!("--{kind}-history"),
                "web=archive"
            ])
            .status
            .success());
    });
}
pub fn browser() {
    let td = tempfile::tempdir().unwrap();
    let source = td.path().join("browser.log");
    fs::write(
        &source,
        (1..1201)
            .map(|n| format!("temperature={n}\n"))
            .collect::<String>(),
    )
    .unwrap();
    let a = App::native(&[
        "--input-file",
        &format!("source={}", source.display()),
        "--set",
        "source.from_start=true",
        "--output-webui",
        "web",
        "--webui-archive",
        &format!("web={}/archive", td.path().display()),
    ]);
    let url = s(&a.control("web", "url", json!({}))["url"]);
    let artifact = root().join("quality/artifacts/browser").join(format!(
        "{}-{}",
        std::env::consts::OS,
        crate::runner::stamp()
    ));
    let memory = App::native(&[
        "--input-file",
        &format!("source={}", source.display()),
        "--set",
        "source.from_start=true",
        "--output-webui",
        "web",
    ]);
    let memory_url = s(&memory.control("web", "url", json!({}))["url"]);
    let r = capture(
        std::process::Command::new("node")
            .arg(root().join("quality/tests/browser/workbench.mjs"))
            .arg(url)
            .arg(&a.state)
            .arg(bin("log-print"))
            .arg(artifact)
            .arg(memory_url)
            .arg(&memory.state)
            .current_dir(root()),
        180,
    );
    print!("{}", String::from_utf8_lossy(&r.stdout));
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}
