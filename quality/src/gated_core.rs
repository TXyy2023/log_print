//! Real Core and TCP, with one explicitly selected archive send barrier.
//! Runs only through `quality fixture gated-core DIR`, never a production binary.
use anyhow::{Context, Result};
use log_proto::{RuntimeConfig, ServerConnection, ServerMessage};
use serde_json::json;
use std::{
    io::Read,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

#[tokio::main(worker_threads = 2)]
pub async fn run(directory: &str) -> Result<()> {
    let directory = Path::new(directory);
    let runtime: RuntimeConfig =
        serde_json::from_slice(&std::fs::read(directory.join("runtime.json"))?)?;
    let core = log_core::Core::new(runtime)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let hit = Arc::new(AtomicBool::new(false));
    crate::support::write_json(
        directory.join("ready.json"),
        &json!({
            "address": listener.local_addr()?.to_string(), "pid": std::process::id(), "transport": "tcp"
        }),
    );
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut byte = [0];
        while matches!(std::io::stdin().read(&mut byte), Ok(n) if n > 0) {}
        let _ = stop.send(());
    });
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            incoming = listener.accept() => {
                let (socket, _) = incoming?;
                let core = core.clone();
                let hit = hit.clone();
                let directory = directory.to_owned();
                tokio::spawn(async move {
                    let connection = ServerConnection::tcp_with_test_send_hook(socket, move |hello, message| {
                        let (welcome, first_record) = match message {
                            ServerMessage::Response { result, error: None, .. }
                                if hello.plugin == "archive" && hello.events && result["subscribed"] == true => (Some(result.clone()), None),
                            ServerMessage::Record { record } if hello.plugin == "archive" && hello.events && record.seq == 1 =>
                                (None, Some(json!({ "stream": record.stream, "epoch": record.epoch, "seq": record.seq }))),
                            _ => (None, None),
                        };
                        let hit = hit.clone();
                        let directory = directory.clone();
                        async move {
                            if let Some(welcome) = welcome {
                                crate::support::write_json(directory.join("subscription.json"), &welcome);
                            }
                            if let Some(record) = first_record {
                                anyhow::ensure!(!hit.swap(true, Ordering::SeqCst), "archive hit the test barrier twice");
                                // Record 1 is on the wire and can be archived. Core awaits
                                // this send's completion before extracting the next batch.
                                // The fixture publishes nothing else until this marker exists.
                                crate::support::write_json(directory.join("gate-ready.json"), &record);
                                let result = tokio::time::timeout(Duration::from_secs(30), async {
                                    while !directory.join("gate-release").exists() {
                                        tokio::time::sleep(Duration::from_millis(5)).await;
                                    }
                                }).await.context("archive send gate was not released within 30s");
                                if let Err(error) = &result {
                                    std::fs::write(directory.join("gate-error"), error.to_string())?;
                                }
                                result?;
                            }
                            Ok(())
                        }
                    }).await;
                    let result = match connection {
                        Ok(connection) => core.serve_connection(connection).await,
                        Err(error) => Err(error),
                    };
                    if let Err(error) = result { eprintln!("gated Core connection: {error:#}"); }
                });
            }
        }
    }
    std::fs::remove_file(directory.join("ready.json"))?;
    Ok(())
}
