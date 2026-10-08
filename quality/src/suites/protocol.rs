use crate::{suites::Suite, support::*};
use serde_json::json;
use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Instant,
};
fn core(transport: &str, options: serde_json::Value) -> Core {
    Core::new(
        transport,
        options,
        json!(
            [{ "id" : "a", "role" : "input", "bin" : "unused", "streams" : [{ "id" :
            "raw", "description" : "stream A" }] }, { "id" : "b", "role" : "input", "bin"
            : "unused", "streams" : [{ "id" : "other" }] }, { "id" : "out", "role" :
            "output", "bin" : "unused", "reads" : ["raw", "other"] }, { "id" :
            "other-out", "role" : "output", "bin" : "unused", "reads" : ["raw"] }]
        ),
    )
}
pub fn run(t: &mut Suite) {
    t.case("unique_ids_receive_order_and_no_saved_semantics", || {
        let c = Core::default();
        let mut a = c.rpc("a");
        let mut b = c.rpc("b");
        let raw = c.stream("a");
        let other = c.stream("b");
        uuid::Uuid::parse_str(&raw).unwrap();
        assert_ne!(raw, other);
        for i in [9, 3, 7] {
            a.publish(&raw, i.to_string().as_bytes(), json!({ "source_seq" : i }));
        }
        b.publish(&other, b"independent", json!({}));
        let rows = c.records("a");
        assert_eq!(
            arr(&rows)
                .iter()
                .map(|v| v["source_seq"].clone())
                .collect::<Vec<_>>(),
            vec![json!(9), json!(3), json!(7)]
        );
        for (i, r) in arr(&rows).iter().enumerate() {
            assert_eq!(r["seq"], i + 1);
            assert!(r.get("durability").is_none())
        }
        assert_eq!(payload(&c.records("b"), None), b"independent");
        assert_eq!(
            c.admin().call("stream.get", json!({ "stream" : raw }))["description"],
            "stream A"
        );
    });
    t.case(
        "rollover_record_limit_and_bytes_are_independent_per_stream",
        || {
            for opts in [
                json!({ "buffer_records" : 3 }),
                json!({ "buffer_bytes" : 1024, "max_payload_bytes" : 1024 }),
            ] {
                let c = core("tcp", opts);
                let mut a = c.rpc("a");
                let mut b = c.rpc("b");
                b.publish(&c.stream("b"), b"keep-me", json!({}));
                for i in 0..20 {
                    a.publish(&c.stream("a"), &[i; 200], json!({}));
                }
                let rows = c.records("a");
                assert!(!arr(&rows).is_empty() && arr(&rows).len() < 20);
                assert_eq!(arr(&rows).last().unwrap()["seq"], 20);
                assert_eq!(payload(&c.records("b"), None), b"keep-me");
            }
        },
    );
    t.case(
        "independent_subscriptions_oldest_then_wait_then_continue",
        || {
            let c = core("tcp", json!({ "buffer_records" : 3 }));
            let mut a = c.rpc("a");
            let stream = c.stream("a");
            for i in 0..5 {
                a.publish(&stream, i.to_string().as_bytes(), json!({}));
            }
            let mut first = c.events("out", true);
            let mut second = c.events("other-out", true);
            for client in [&mut first, &mut second] {
                client.call("subscribe", json!({ "stream" : stream }));
                for seq in 3..6 {
                    assert_eq!(client.receive().unwrap()["record"]["seq"], seq);
                }
            }
            pause(150);
            a.publish(&stream, b"later", json!({}));
            for client in [&mut first, &mut second] {
                assert_eq!(
                    client.receive().unwrap()["record"]["payload"],
                    json!(b"later".as_slice())
                );
            }
        },
    );
    t.case("empty_subscription_receives_future_data", || {
        let c = Core::default();
        let mut a = c.rpc("a");
        let mut out = c.events("out", true);
        let stream = c.stream("a");
        out.call("subscribe", json!({ "stream" : stream }));
        pause(100);
        a.publish(&stream, b"first", json!({}));
        assert_eq!(out.receive().unwrap()["record"]["seq"], 1);
    });
    t.case("single_writer_and_foreign_stream_rejected", || {
        let c = Core::default();
        let mut a = c.rpc("a");
        assert!(Rpc::connect(&c.address, "a", "a", "tcp", false, "log-print/2").is_err());
        for id in ["b", "out"] {
            assert!(c
                .rpc(id)
                .try_call(
                    "publish",
                    json!({ "stream" : c.stream("a"),
                    "payload" : b"forbidden", "key" : "key" })
                )
                .is_err());
        }
        a.publish(&c.stream("a"), b"owned", json!({}));
    });
    t.case(
        "owner_disconnect_keeps_memory_and_does_not_close_stream",
        || {
            let c = Core::default();
            c.rpc("a").publish(&c.stream("a"), b"buffered", json!({}));
            pause(100);
            assert_eq!(payload(&c.records("a"), None), b"buffered");
            let mut out = c.events("out", true);
            out.call("subscribe", json!({ "stream" : c.stream("a") }));
            assert_eq!(
                out.receive().unwrap()["record"]["payload"],
                json!(b"buffered".as_slice())
            );
        },
    );
    t.case("core_restart_has_no_history_and_new_identity", || {
        let mut c = Core::default();
        let original = c.stream("a");
        c.rpc("a").publish(&original, b"not-persistent", json!({}));
        c.stop();
        c.start();
        let current = c.stream("a");
        assert_ne!(current, original);
        assert_eq!(c.records("a"), json!([]));
        assert!(c
            .admin()
            .try_call("stream.get", json!({ "stream" : original }))
            .is_err());
        for file in std::fs::read_dir(c.path()).unwrap() {
            let name = file.unwrap().file_name().to_string_lossy().into_owned();
            assert!(!name.contains("sqlite") && !name.ends_with(".db"));
        }
    });
    t.case(
        "wrong_token_old_protocol_and_unknown_identity_rejected",
        || {
            let c = Core::default();
            for (id, token, protocol) in [
                ("a", "wrong", "log-print/2"),
                ("unknown", "a", "log-print/2"),
                ("a", "a", "log-print/1"),
            ] {
                assert!(Rpc::connect(&c.address, id, token, "tcp", false, protocol).is_err())
            }
            assert!(c.admin().call("status", json!({}))["streams"].is_array());
        },
    );
    t.case("invalid_publish_has_no_partial_record", || {
        let c = core("tcp", json!({ "max_payload_bytes" : 100 }));
        let mut a = c.rpc("a");
        for data in [json!([256]), json!(vec![120; 101]), json!("not-bytes")] {
            assert!(a
                .try_call(
                    "publish",
                    json!({ "stream" : c.stream("a"), "payload" :
                    data, "key" : "bad" })
                )
                .is_err());
        }
        assert_eq!(c.records("a"), json!([]));
        a.publish(&c.stream("a"), b"ok", json!({}));
        assert_eq!(c.records("a")[0]["seq"], 1);
    });
    t.case(
        "unread_slow_subscriber_does_not_block_other_stream_or_input",
        || {
            let c = core("tcp", json!({ "buffer_records" : 8 }));
            let mut a = c.rpc("a");
            let mut b = c.rpc("b");
            let mut slow = c.events("out", true);
            slow.call("subscribe", json!({ "stream" : c.stream("a") }));
            let before = Instant::now();
            for i in 0..300 {
                a.publish(
                    &c.stream("a"),
                    &vec![120; 8192],
                    json!({ "key" : i.to_string() }),
                );
            }
            b.publish(&c.stream("b"), b"still-responsive", json!({}));
            assert!(before.elapsed().as_secs() < 12);
            assert_eq!(payload(&c.records("b"), None), b"still-responsive");
            assert!(arr(&c.records("a")).len() <= 8);
        },
    );
    t.case("invalid_frame_does_not_take_down_core", || {
        let c = Core::default();
        for data in [b"{invalid}\n".to_vec(), vec![120; 1024 * 1024 + 1]] {
            let mut sock = TcpStream::connect(&c.address).unwrap();
            sock.set_read_timeout(Some(std::time::Duration::from_secs(6)))
                .unwrap();
            sock.set_write_timeout(Some(std::time::Duration::from_secs(6)))
                .unwrap();
            let _ = sock.write_all(&data);
            match sock.read(&mut [0; 1024]) {
                Ok(n) => assert_eq!(n, 0),
                Err(e) => {
                    assert!(
                        matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                        ),
                        "{e}"
                    )
                }
            }
        }
        assert!(c.admin().call("status", json!({}))["streams"].is_array());
    });
    t.case("udp_registration_publish_no_ack_and_subscription", || {
        let c = core("udp", json!({}));
        let mut a = c.rpc("a");
        let mut out = c.events("out", true);
        assert_eq!(a.welcome["protocol"], "log-print/2");
        out.call("subscribe", json!({ "stream" : c.stream("a") }));
        a.publish(&c.stream("a"), b"udp", json!({ "source_seq" : 42 }));
        let r = out.receive().unwrap();
        assert_eq!(r["record"]["payload"], json!(b"udp".as_slice()));
        assert_eq!(r["record"]["source_seq"], 42);
        a.timeout(150);
        assert!(a.receive().is_err());
        assert_eq!(payload(&c.records("a"), None), b"udp");
    });
    t.case("udp_bad_registration_does_not_claim_writer", || {
        let c = core("udp", json!({}));
        assert!(Rpc::connect(&c.address, "a", "wrong", "udp", false, "log-print/2").is_err());
        assert_eq!(c.rpc("a").welcome["protocol"], "log-print/2");
    });
}
