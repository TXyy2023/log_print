use super::*;
use log_proto::{read_json, write_json};
use tokio::{
    io::BufReader,
    net::{TcpListener, TcpStream, UdpSocket},
};
fn welcome() -> ServerMessage {
    ServerMessage::Response {
        id: 0,
        result: json!({"protocol":PROTOCOL,"stream":{"id":"stream-id","alias":"s"},"reads":["stream-id"]}),
        error: None,
    }
}
fn record() -> Record {
    serde_json::from_value(json!({"stream":"stream-id","epoch":"e","seq":3,"key":"k","payload":[0,255],"observed_ts_ns":1,"source_seq":7,"channel":"stdout","upstream":{},"upstream_epochs":{}})).unwrap()
}
async fn handshake(
    stream: TcpStream,
    events: bool,
) -> (
    BufReader<tokio::net::tcp::OwnedReadHalf>,
    tokio::net::tcp::OwnedWriteHalf,
) {
    let (r, mut w) = stream.into_split();
    let mut r = BufReader::new(r);
    let hello: Hello = read_json(&mut r).await.unwrap().unwrap();
    assert_eq!(hello.protocol, PROTOCOL);
    assert_eq!(hello.events, events);
    write_json(&mut w, &welcome()).await.unwrap();
    (r, w)
}
#[tokio::test]
async fn tcp_publish_reports_core_acceptance_and_preserves_tags() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = tokio::spawn(async move {
        let (mut r, mut w) = handshake(listener.accept().await.unwrap().0, false).await;
        let request: Request = read_json(&mut r).await.unwrap().unwrap();
        assert_eq!(request.op, "publish");
        assert_ne!(request.id, 0);
        assert_eq!(request.args["stream"], "stream-id");
        assert_eq!(request.args["channel"], "stdout");
        assert_eq!(request.args["source_seq"], 7);
        write_json(
            &mut w,
            &ServerMessage::Response {
                id: request.id,
                result: serde_json::to_value(record()).unwrap(),
                error: None,
            },
        )
        .await
        .unwrap();
    });
    let (client, _, _) = Client::connect(&addr, "p", "token").await.unwrap();
    assert_eq!(client.stream_id(), Some("stream-id"));
    assert_eq!(client.read_streams(), ["stream-id"]);
    let result = client
        .publish_tagged(
            "s",
            "k",
            vec![0, 255],
            None,
            BTreeMap::new(),
            Some("stdout".into()),
            Some(7),
        )
        .await
        .unwrap();
    assert!(matches!(result,PublishOutcome::Accepted(r) if *r==record()));
    server.await.unwrap();
}
#[tokio::test]
async fn udp_registers_then_sends_without_per_record_ack() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap().to_string();
    let (registered, registration) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut bytes = vec![0; log_proto::MAX_DATAGRAM];
        let (n, peer) = socket.recv_from(&mut bytes).await.unwrap();
        let hello: Hello = serde_json::from_slice(&bytes[..n]).unwrap();
        assert_eq!(hello.protocol, PROTOCOL);
        socket
            .send_to(&serde_json::to_vec(&welcome()).unwrap(), peer)
            .await
            .unwrap();
        registered.send(()).unwrap();
        for _ in 0..3 {
            let (n, p) = socket.recv_from(&mut bytes).await.unwrap();
            assert_eq!(p, peer);
            let request: Request = serde_json::from_slice(&bytes[..n]).unwrap();
            assert_eq!(request.id, 0);
            assert_eq!(request.op, "publish");
        }
    });
    let (client, _, _) =
        Client::connect_with_transport(&addr, "p", "token", json!({}), TransportKind::Udp)
            .await
            .unwrap();
    registration.await.unwrap();
    for _ in 0..3 {
        let outcome = tokio::time::timeout(
            Duration::from_millis(300),
            client.publish("s", "k", vec![1], None, BTreeMap::new()),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(outcome, PublishOutcome::LocalSent));
    }
    server.await.unwrap();
}
#[tokio::test]
async fn core_rejection_is_returned_without_retry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = tokio::spawn(async move {
        let (mut r, mut w) = handshake(listener.accept().await.unwrap().0, false).await;
        let request: Request = read_json(&mut r).await.unwrap().unwrap();
        write_json(
            &mut w,
            &ServerMessage::Response {
                id: request.id,
                result: Value::Null,
                error: Some(Fault {
                    code: "writer_occupied".into(),
                    message: "one writer".into(),
                }),
            },
        )
        .await
        .unwrap();
    });
    let (client, _, _) = Client::connect(&addr, "p", "token").await.unwrap();
    let error = client
        .publish("s", "k", vec![], None, BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<Fault>().unwrap().code,
        "writer_occupied"
    );
    server.await.unwrap();
}
#[tokio::test]
async fn subscriptions_start_at_retained_oldest_and_wait_at_tail() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (next, wait) = oneshot::channel();
    let (consumed, finish) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (_control_r, _control_w) = handshake(listener.accept().await.unwrap().0, false).await;
        let (mut r, mut w) = handshake(listener.accept().await.unwrap().0, true).await;
        let request: Request = read_json(&mut r).await.unwrap().unwrap();
        assert_eq!(request.args, json!({"stream":"stream-id"}));
        write_json(
            &mut w,
            &ServerMessage::Response {
                id: 1,
                result: json!({"stream":"stream-id","epoch":"e","from":3}),
                error: None,
            },
        )
        .await
        .unwrap();
        write_json(&mut w, &ServerMessage::Record { record: record() })
            .await
            .unwrap();
        wait.await.unwrap();
        let mut later = record();
        later.seq = 4;
        write_json(&mut w, &ServerMessage::Record { record: later })
            .await
            .unwrap();
        finish.await.unwrap();
    });
    let (client, mut events, _) = Client::connect(&addr, "p", "token").await.unwrap();
    assert_eq!(client.subscribe("s").await.unwrap()["from"], 3);
    assert!(matches!(events.recv().await,Some(Event::Record(r)) if r.seq==3));
    assert!(
        tokio::time::timeout(Duration::from_millis(25), events.recv())
            .await
            .is_err()
    );
    next.send(()).unwrap();
    assert!(matches!(events.recv().await,Some(Event::Record(r)) if r.seq==4));
    consumed.send(()).unwrap();
    server.await.unwrap();
}
#[tokio::test]
async fn cancelled_requests_remove_pending_correlation_and_leave_followup_usable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let (seen, wait) = oneshot::channel();
    let (resume, go) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut r, mut w) = handshake(listener.accept().await.unwrap().0, false).await;
        let request: Request = read_json(&mut r).await.unwrap().unwrap();
        assert_eq!(request.args["body"].as_str().unwrap().len(), 200000);
        seen.send(()).unwrap();
        go.await.unwrap();
        let next: Request = read_json(&mut r).await.unwrap().unwrap();
        write_json(
            &mut w,
            &ServerMessage::Response {
                id: next.id,
                result: json!({"ok":true}),
                error: None,
            },
        )
        .await
        .unwrap();
    });
    let (client, _, _) = Client::connect(&addr, "p", "token").await.unwrap();
    let c = client.clone();
    let cancelled =
        tokio::spawn(async move { c.request("slow", json!({"body":"x".repeat(200000)})).await });
    wait.await.unwrap();
    cancelled.abort();
    let _ = cancelled.await;
    assert!(client.inner.pending.lock().unwrap().is_empty());
    resume.send(()).unwrap();
    assert_eq!(client.request("next", json!({})).await.unwrap()["ok"], true);
    server.await.unwrap();
}
#[tokio::test]
async fn ordinary_rpc_budget_does_not_consume_the_control_reply_reserve() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let server = tokio::spawn(async move {
        let (mut r, mut w) = handshake(listener.accept().await.unwrap().0, false).await;
        let request: Request = read_json(&mut r).await.unwrap().unwrap();
        assert_eq!(request.op, "reply");
        write_json(
            &mut w,
            &ServerMessage::Response {
                id: request.id,
                result: json!({"ok":true}),
                error: None,
            },
        )
        .await
        .unwrap();
    });
    let (client, _, _) = Client::connect(&address, "p", "token").await.unwrap();
    let _permits = client.inner.slots.acquire_many(32).await.unwrap();
    let c = client.clone();
    let waiting = tokio::spawn(async move { c.request("ordinary", json!({})).await });
    tokio::task::yield_now().await;
    assert!(client.inner.pending.lock().unwrap().is_empty());
    tokio::time::timeout(
        Duration::from_secs(1),
        client.reply_control(1, json!({"stopping":true}), None),
    )
    .await
    .unwrap()
    .unwrap();
    waiting.abort();
    let _ = waiting.await;
    server.await.unwrap();
}
