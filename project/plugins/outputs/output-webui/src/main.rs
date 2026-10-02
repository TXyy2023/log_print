mod engine;
mod lines;
mod store;
use anyhow::{bail, Context, Result};
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use engine::Engine;
use include_dir::{include_dir, Dir};
use log_plugin_sdk::Event as CoreEvent;
use log_proto::{Fault, TransportKind};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet, convert::Infallible, future::IntoFuture, net::SocketAddr, path::PathBuf,
    sync::Arc, time::Duration,
};
static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets");
#[derive(Deserialize)]
struct Command {
    method: String,
    #[serde(default)]
    args: Value,
}
fn host(headers: &HeaderMap, engine: &Engine) -> bool {
    headers.get(header::HOST).and_then(|v| v.to_str().ok())
        == Some(
            engine
                .url
                .trim_start_matches("http://")
                .trim_end_matches('/'),
        )
}
async fn state(State(engine): State<Arc<Engine>>, headers: HeaderMap) -> Response {
    if !host(&headers, &engine) {
        return StatusCode::FORBIDDEN.into_response();
    }
    Json(engine.state()).into_response()
}
async fn control(
    State(engine): State<Arc<Engine>>,
    headers: HeaderMap,
    Json(command): Json<Command>,
) -> Response {
    if !host(&headers, &engine)
        || headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
            != Some(engine.url.trim_end_matches('/'))
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"same-origin request required"})),
        )
            .into_response();
    }
    match engine
        .command(&command.method, command.args, 4 * 1024 * 1024)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(e) => {
            let error = format!("{e:#}");
            let code = if error.starts_with("revision_conflict") {
                StatusCode::CONFLICT
            } else if error.starts_with("busy") {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::BAD_REQUEST
            };
            (code, Json(json!({"error":error}))).into_response()
        }
    }
}
async fn data(
    State(engine): State<Arc<Engine>>,
    headers: HeaderMap,
    axum::extract::Query(args): axum::extract::Query<std::collections::BTreeMap<String, String>>,
) -> Response {
    if !host(&headers, &engine) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let value = args
        .into_iter()
        .map(|(k, v)| (k, serde_json::from_str(&v).unwrap_or_else(|_| json!(v))))
        .collect::<serde_json::Map<_, _>>();
    match engine
        .command("query.get", Value::Object(value), 4 * 1024 * 1024)
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":format!("{e:#}")})),
        )
            .into_response(),
    }
}
async fn sse(State(engine): State<Arc<Engine>>, headers: HeaderMap) -> Response {
    if !host(&headers, &engine) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut rx = engine.updates.subscribe();
    Sse::new(async_stream::stream! {yield Ok::<_,Infallible>(Event::default().event("state").json_data(engine.state()).unwrap());while let Ok(())|Err(tokio::sync::broadcast::error::RecvError::Lagged(_))=rx.recv().await {yield Ok(Event::default().event("state").json_data(engine.state()).unwrap())}}).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))).into_response()
}
async fn asset(Path(path): Path<String>) -> Response {
    static_asset(&path)
}
async fn index() -> Response {
    static_asset("index.html")
}
fn static_asset(path: &str) -> Response {
    let Some(file) = ASSETS.get_file(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content = if path.ends_with(".js") {
        "text/javascript"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".json") {
        "application/json"
    } else {
        "text/html; charset=utf-8"
    };
    ([(header::CONTENT_TYPE,content),(header::CONTENT_SECURITY_POLICY,"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self' data:; object-src 'none'; frame-ancestors 'none'"),(header::X_CONTENT_TYPE_OPTIONS,"nosniff")],file.contents()).into_response()
}
#[tokio::main]
async fn main() -> Result<()> {
    let (client, mut events, mut controls) = log_plugin_sdk::connect_env().await?;
    let mut config = client.config().clone();
    let address: SocketAddr = config["listen"].as_str().unwrap_or("127.0.0.1:0").parse()?;
    if !address.ip().is_loopback() {
        bail!("WebUI must listen on loopback");
    }
    if config.get("state_path").is_none() {
        config["state_path"] = json!(PathBuf::from(".webui")
            .join(std::env::var("LOG_PRINT_PLUGIN")?)
            .join("pages.sqlite3"));
    }
    let listener = tokio::net::TcpListener::bind(address).await?;
    let url = format!("http://{}", listener.local_addr()?);
    let store = store::Store::open(&PathBuf::from(
        config["state_path"].as_str().context("state_path")?,
    ))?;
    let scratch = PathBuf::from(config["state_path"].as_str().context("state_path")?)
        .parent()
        .context("state directory")?
        .join("queries");
    if scratch.is_dir() {
        for file in std::fs::read_dir(&scratch)? {
            let file = file?;
            if file.path().extension().is_some_and(|s| s == "jsonl")
                && file
                    .path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            {
                std::fs::remove_file(file.path())?;
            }
        }
    }
    let engine = Arc::new(Engine::new(client.clone(), store, url.clone(), config));
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let ingestion = engine.clone();
    let mut ingest_stop = shutdown.clone();
    let ingest = tokio::spawn(async move {
        let mut subscribed = BTreeSet::new();
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        loop {
            tokio::select! {
                _=ingest_stop.changed()=>break,
                _ = tick.tick() => {
                    let catalog = ingestion.client.streams().await?;
                    let list = catalog.as_array().context("catalog")?.clone();
                    for entry in &list {
                        let id = entry["id"].as_str().context("stream id")?;
                        if !subscribed.contains(id) {
                            ingestion.client.subscribe(id).await?;
                            subscribed.insert(id.to_owned());
                        }
                    }
                    *ingestion.catalog.lock().unwrap() = list;
                    if let Some(plugin) = ingestion.config["history_plugin"].as_str() {
                        let status = ingestion.client.request("plugin.status", json!({"plugin":plugin})).await
                            .unwrap_or_else(|e| json!({"state":"unavailable","error":format!("{e:#}")}));
                        *ingestion.archive_writer.lock().unwrap() = status;
                    }
                    let _ = ingestion.updates.send(());
                },
                event=events.recv()=>match event {Some(CoreEvent::Record(r))=>ingestion.cache.lock().unwrap().push(r),Some(CoreEvent::Gap {stream,from,to,reason,..})=>{ingestion.cache.lock().unwrap().errors.insert(stream,format!("gap {from}..{to}: {reason}"));},Some(CoreEvent::Disconnected {stream,reason})=>{ingestion.cache.lock().unwrap().errors.insert(stream,reason);},None=>bail!("Core event channel closed")}
            }
        }
        Ok::<(), anyhow::Error>(())
    });
    let ctl = engine.clone();
    let ctl_stop = stop.clone();
    let controller = tokio::spawn(async move {
        while let Some(control) = controls.recv().await {
            let method = control.method.as_str();
            let result = if method == "shutdown" {
                ctl_stop.send_replace(true);
                Ok(json!({"stopping":true}))
            } else if method == "config.get" {
                Ok(json!({"effective":ctl.config,"dynamic_fields":["pages"]}))
            } else {
                let budget = if ctl.client.transport() == TransportKind::Udp {
                    log_proto::MAX_DATAGRAM - 4096
                } else {
                    log_proto::MAX_WIRE / 2
                };
                ctl.command(method, control.args, budget).await
            };
            let result = result.and_then(|mut value| {
                if value.get("state").is_some() && value.get("revision").is_some() {
                    value.as_object_mut().unwrap().remove("state");
                    value["committed"] = json!(true);
                }
                let budget = if ctl.client.transport() == TransportKind::Udp {
                    log_proto::MAX_DATAGRAM - 4096
                } else {
                    log_proto::MAX_WIRE / 2
                };
                if serde_json::to_vec(&value)?.len() > budget {
                    bail!("reply exceeds frame budget; use HTTP or request a smaller page");
                }
                Ok(value)
            });
            let (value, error) = match result {
                Ok(v) => (v, None),
                Err(e) => (
                    Value::Null,
                    Some(Fault {
                        code: if e.to_string().starts_with("revision_conflict") {
                            "revision_conflict"
                        } else {
                            "webui_error"
                        }
                        .into(),
                        message: format!("{e:#}"),
                    }),
                ),
            };
            ctl.client
                .reply_control(control.call_id, value, error)
                .await?;
            if method == "shutdown" {
                break;
            }
        }
        ctl_stop.send_replace(true);
        Ok::<(), anyhow::Error>(())
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/api/state", get(state))
        .route("/api/control", post(control))
        .route("/api/events", get(sse))
        .route("/api/data", get(data))
        .route("/{*path}", get(asset))
        .layer(DefaultBodyLimit::max(512 * 1024))
        .with_state(engine.clone());
    client.request("report",json!({"state":"serving","url":url,"archive_enabled":engine.config["history_path"].is_string()})).await?;
    eprintln!("[output-webui] ready {url}");
    let mut server_stop = shutdown.clone();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = server_stop.changed().await;
        })
        .into_future();
    tokio::pin!(server);
    let result = tokio::select! {r=&mut server=>r.map_err(Into::into),r=ingest=>r.context("ingest task")?,_=log_plugin_sdk::termination()=>Ok(())};
    stop.send_replace(true);
    engine.cancel_all();
    controller.abort();
    log_plugin_sdk::finish(&client, &result).await;
    result
}
