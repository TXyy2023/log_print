use crate::{suites::Suite, support::*};
use serde_json::{json, Value};
use std::{fs, time::Instant};
pub fn source() -> Value {
    json!(
        { "id" : "source", "role" : "input", "bin" : "unused", "streams" : [{ "id" :
        "raw" }] }
    )
}
pub fn output(binary: &str, config: Value, derived: bool) -> Value {
    json!(
        { "id" : "sink", "role" : "output", "bin" : binary, "reads" : ["raw"], "streams"
        : if derived { json!([{ "id" : "derived", "parents" : ["raw"] }]) } else {
        json!([]) }, "config" : config }
    )
}
pub fn core(p: &Value, opts: Value) -> Core {
    Core::new("tcp", opts, json!([source(), p]))
}
pub fn publish(c: &Core, w: &mut Rpc, data: &[u8], seq: u64, extra: Value) -> Value {
    let mut x = json!({ "key" : format!("fixture:{seq}"), "source_seq" : seq });
    x.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    w.publish(&c.stream("source"), data, x)
}
pub fn run(t: &mut Suite) {
    t.case("raw_dynamic_uuid_attachment_preserves_binary", || {
        let mut p = output(
            "output-raw",
            json!({ "streams" : ["obsolete-config-value"] }),
            false,
        );
        p["reads"] = json!([]);
        let c = core(&p, json!({}));
        let mut w = c.rpc("source");
        publish(&c, &mut w, b"\0\xfffirst\n", 1, json!({}));
        c.admin().call(
            "plugin.attach",
            json!({ "plugin" : "sink", "stream" : c.stream("source") }),
        );
        let mut run = Plugin::new(&c, &p, &[]);
        run.ready("displaying");
        publish(&c, &mut w, b"second", 2, json!({}));
        until(|| bytes(&run.stdout) == b"\0\xfffirst\nsecond");
        run.stop();
        assert_eq!(bytes(&run.stdout), b"\0\xfffirst\nsecond");
    });
    for format in ["raw", "jsonl"] {
        let name = format!("{format}_and_sqlite_roundtrip_complete_metadata");
        t.case(
            &name,
            || {
                let td = tempfile::tempdir().unwrap();
                let path = td.path().join("data");
                let database = td.path().join("data.sqlite");
                let p = output(
                    "output-file",
                    json!(
                        { "streams" : ["raw"], "mode" : "create", "file" : { "format" :
                        format, "paths" : { "raw" : path } }, "sqlite" : { "path" :
                        database }, "commit" : { "max_records" : 2, "max_bytes" : 4096,
                        "max_delay_ms" : 30 } }
                    ),
                    false,
                );
                let c = core(&p, json!({}));
                let mut w = c.rpc("source");
                let mut run = Plugin::new(&c, &p, &[]);
                run.ready("archiving");
                let expected: Vec<_> = [b"\0\xff\n".as_slice(), b"", "中".as_bytes()]
                    .iter()
                    .enumerate()
                    .map(|(i, b)| publish(
                        &c,
                        &mut w,
                        b,
                        i as u64 + 1,
                        json!({ "channel" : "stdout", "source_ts_ns" : u64::MAX }),
                    ))
                    .collect();
                until(|| {
                    c
                        .admin()
                        .call(
                            "control",
                            json!({ "target" : "sink", "method" : "status.get" }),
                        )["common"][c.stream("source")]["next"] == 4
                });
                assert_eq!(run.stop() ["stopped"], true);
                if format == "raw" {
                    assert_eq!(bytes(& path), b"\0\xff\n\xe4\xb8\xad");
                } else {
                    let rows: Vec<Value> = text(&path)
                        .lines()
                        .map(|s| serde_json::from_str(s).unwrap())
                        .collect();
                    assert!(rows.iter().all(| r | r["format_version"] == 2));
                    assert_eq!(
                        rows.iter().map(| r | r["record"].clone()).collect::< Vec < _ >>
                        (), expected
                    );
                }
                let db = rusqlite::Connection::open(database).unwrap();
                let rows: Vec<(Vec<u8>, String, String, String)> = db
                    .prepare(
                        "SELECT payload,source_ts_ns,channel,source_seq FROM records ORDER BY length(seq),seq",
                    )
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                for (i, (bytes, ts, ch, seq)) in rows.iter().enumerate() {
                    assert_eq!(json!(bytes), expected[i] ["payload"]);
                    assert_eq!(ts,& u64::MAX.to_string());
                    assert_eq!(ch, "stdout");
                    assert_eq!(seq,& (i + 1).to_string());
                }
                assert_eq!(rows.len(), 3);
                assert_eq!(
                    db.query_row("PRAGMA integrity_check", [],| r | r.get::< _, String >
                    (0)).unwrap(), "ok"
                );
                assert!(bytes(& run.stdout).is_empty());
            },
        );
    }
    t.case("archive_refuses_existing_targets_without_overwrite", || {
        let td = tempfile::tempdir().unwrap();
        let path = td.path().join("existing");
        fs::write(&path, b"keep-me").unwrap();
        let p = output(
            "output-file",
            json!(
                { "streams" : ["raw"], "mode" : "create", "file" : { "format" :
                "raw", "paths" : { "raw" : path } } }
            ),
            false,
        );
        let c = core(&p, json!({}));
        let mut run = Plugin::new(&c, &p, &[]);
        assert_eq!(run.wait(false)["state"], "failed");
        assert_eq!(bytes(path), b"keep-me");
    });
    t.case("archive_sync_failure_does_not_report_saved", || {
        let td = tempfile::tempdir().unwrap();
        let p = output(
            "output-file",
            json!(
                { "streams" : ["raw"], "mode" : "create", "file" : { "format" :
                "raw", "paths" : { "raw" : td.path().join("data") } }, "commit" : {
                "max_records" : 1, "max_bytes" : 4096, "max_delay_ms" : 20 } }
            ),
            false,
        );
        let c = core(&p, json!({}));
        let mut w = c.rpc("source");
        let mut run = Plugin::new(
            &c,
            &p,
            &[
                ("LOG_PRINT_ARCHIVE_TESTING", "1"),
                ("LOG_PRINT_ARCHIVE_ERRORPOINT", "file_sync"),
            ],
        );
        run.ready("archiving");
        publish(&c, &mut w, b"unconfirmed", 1, json!({}));
        let r = run.wait(false);
        assert_eq!(r["state"], "failed");
        assert_eq!(r["complete"], false);
        assert!(s(&r["error"]).contains("file_sync"));
    });
    t.case(
        "transform_number_timestamp_reorders_and_leaves_original_unchanged",
        || {
            let p = output(
                "output-transform",
                json!(
                    { "streams" : ["raw"], "output_stream" : "derived", "number" : true,
                    "timestamp" : true, "reorder" : true, "max_delay_ms" : 1000 }
                ),
                true,
            );
            let c = core(&p, json!({}));
            let mut w = c.rpc("source");
            let mut run = Plugin::new(&c, &p, &[]);
            run.ready("transforming");
            let original: Vec<_> = [(3, b"three".as_slice()), (1, b"one"), (2, b"two")]
                .iter()
                .map(|(seq, b)| publish(&c, &mut w, b, *seq, json!({ "source_ts_ns" : 100 + seq })))
                .collect();
            until(|| arr(&c.records("sink")).len() == 3);
            assert_eq!(
                payload(&c.records("sink"), None),
                b"[n=1] [ts_ns=101] one[n=2] [ts_ns=102] two[n=3] [ts_ns=103] three"
            );
            assert_eq!(c.records("source"), json!(original));
            assert_ne!(c.stream("source"), c.stream("sink"));
            run.stop();
        },
    );
    t.case(
        "transform_missing_sequence_timeout_duplicate_and_shutdown_flush",
        || {
            let p = output(
                "output-transform",
                json!({ "reorder" : true, "max_delay_ms" : 120 }),
                true,
            );
            let c = core(&p, json!({}));
            let mut w = c.rpc("source");
            let mut run = Plugin::new(&c, &p, &[]);
            run.ready("transforming");
            publish(&c, &mut w, b"three", 3, json!({}));
            publish(&c, &mut w, b"duplicate", 3, json!({}));
            until(|| arr(&c.records("sink")).len() == 1);
            publish(&c, &mut w, b"late", 1, json!({}));
            publish(&c, &mut w, b"seven", 7, json!({}));
            until(|| arr(&c.records("source")).len() == 4);
            until(|| arr(&c.records("sink")).len() == 2);
            run.stop();
            assert_eq!(payload(&c.records("sink"), None), b"threeseven");
            let r = run.report();
            assert_eq!(r["state"], "stopped");
            assert_eq!(r["duplicates"], 2);
            assert_eq!(r["pending"], 0);
        },
    );
    t.case("transform_shutdown_flushes_a_confirmed_pending_gap", || {
        let p = output(
            "output-transform",
            json!({ "reorder" : true, "max_delay_ms" : 60000 }),
            true,
        );
        let c = core(&p, json!({}));
        let mut w = c.rpc("source");
        let mut run = Plugin::new(&c, &p, &[]);
        run.ready("transforming");
        publish(
            &c,
            &mut w,
            b"tenth-pending",
            10,
            json!({ "channel" : "stdout" }),
        );
        publish(&c, &mut w, b"barrier", 1, json!({ "channel" : "stderr" }));
        until(|| arr(&c.records("sink")).len() == 1);
        run.stop();
        assert_eq!(payload(&c.records("sink"), None), b"barriertenth-pending");
        assert_eq!(run.report()["skipped_source_sequences"], 9);
    });
    t.case(
        "archive_late_subscription_starts_at_retained_oldest",
        || {
            let td = tempfile::tempdir().unwrap();
            let path = td.path().join("retained");
            let p = output(
                "output-file",
                json!(
                    { "streams" : ["raw"], "mode" : "create", "file" : { "format" :
                    "raw", "paths" : { "raw" : path } } }
                ),
                false,
            );
            let c = core(&p, json!({ "buffer_records" : 2 }));
            let mut w = c.rpc("source");
            for seq in 1..6 {
                publish(&c, &mut w, seq.to_string().as_bytes(), seq, json!({}));
            }
            let mut run = Plugin::new(&c, &p, &[]);
            run.ready("archiving");
            until(|| bytes(&path) == b"45");
            run.stop();
            assert_eq!(bytes(path), b"45");
            assert_eq!(run.report()["common"][c.stream("source")]["next"], 6);
        },
    );
    t.case(
        "transform_stdout_stderr_source_sequences_are_independent",
        || {
            let p = output(
                "output-transform",
                json!({ "reorder" : true, "max_delay_ms" : 1000 }),
                true,
            );
            let c = core(&p, json!({}));
            let mut w = c.rpc("source");
            let mut run = Plugin::new(&c, &p, &[]);
            run.ready("transforming");
            for (b, seq, ch) in [
                ("out2", 2, "stdout"),
                ("err1", 1, "stderr"),
                ("out1", 1, "stdout"),
            ] {
                publish(&c, &mut w, b.as_bytes(), seq, json!({ "channel" : ch }));
            }
            until(|| arr(&c.records("sink")).len() == 3);
            run.stop();
            let rows = c.records("sink");
            assert_eq!(payload(&rows, None), b"err1out1out2");
            for (i, (ch, seq)) in [("stderr", 1), ("stdout", 1), ("stdout", 2)]
                .iter()
                .enumerate()
            {
                assert_eq!(rows[i]["channel"], *ch);
                assert_eq!(rows[i]["source_seq"], *seq);
            }
        },
    );
    t.case(
        "transform_channel_limit_applies_without_reorder_or_source_sequence",
        || {
            for (reorder, sequence) in [(false, true), (true, false)] {
                let p = output(
                    "output-transform",
                    json!({ "reorder" : reorder, "max_channels" : 1 }),
                    true,
                );
                let c = core(&p, json!({}));
                let mut w = c.rpc("source");
                let mut run = Plugin::new(&c, &p, &[]);
                run.ready("transforming");
                let mut x = json!({ "channel" : "first" });
                if sequence {
                    x["source_seq"] = json!(1);
                }
                w.publish(&c.stream("source"), b"first", x.clone());
                until(|| arr(&c.records("sink")).len() == 1);
                x["channel"] = json!("second");
                w.publish(&c.stream("source"), b"over-limit", x);
                let r = run.wait(false);
                assert_eq!(r["state"], "failed");
                assert!(s(&r["error"]).contains("channel state limit"));
                assert_eq!(payload(&c.records("sink"), None), b"first");
            }
        },
    );
    t.case(
        "transform_expiry_uses_record_deadline_without_an_extra_timer_period",
        || {
            let p = output(
                "output-transform",
                json!({ "reorder" : true, "max_delay_ms" : 2000 }),
                true,
            );
            let c = core(&p, json!({}));
            let mut w = c.rpc("source");
            let mut run = Plugin::new(&c, &p, &[]);
            run.ready("transforming");
            pause(150);
            let started = Instant::now();
            publish(&c, &mut w, b"gap-at-two", 2, json!({}));
            eventually(5, || (!arr(&c.records("sink")).is_empty()).then_some(()));
            assert!(started.elapsed().as_secs_f64() < 3.2);
            run.stop();
        },
    );
}
