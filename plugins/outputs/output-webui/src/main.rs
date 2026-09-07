use anyhow::{ensure, Result};
use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive},
        Html, IntoResponse, Response, Sse,
    },
    routing::{get, post},
    Json, Router,
};
use log_plot::{lock, OutputConfig, PlotHub, SharedHub};
use log_plugin_sdk::Event;
use log_proto::Fault;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};
use tokio::sync::{watch, Semaphore};
#[derive(Clone)]
struct App {
    hub: SharedHub,
    connections: Arc<Semaphore>,
    exports: Arc<Semaphore>,
    stop: watch::Receiver<bool>,
}
type ApiResult<T> = std::result::Result<T, ApiError>;
struct ApiError(anyhow::Error);
impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let msg = self.0.to_string();
        let status = if msg.starts_with("revision_conflict") {
            StatusCode::CONFLICT
        } else {
            StatusCode::BAD_REQUEST
        };
        (status, Json(json!({"error":msg}))).into_response()
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let (client, mut events, mut controls) = log_plugin_sdk::connect_env().await?;
    let config = OutputConfig::from_value(client.config())?;
    let bind: SocketAddr = config.web_bind.parse()?;
    ensure!(
        bind.ip().is_loopback(),
        "WebUI only supports loopback bind addresses"
    );
    let hub = PlotHub::shared(config.clone())?;
    for stream in &config.streams {
        client.subscribe(stream, config.from).await?;
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let address = listener.local_addr()?;
    let url = format!("http://{address}");
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let data_hub = hub.clone();
    let data_stop = stop_tx.clone();
    let data_task = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Ok(mut h) = lock(&data_hub) {
                match event {
                    Event::Record(r) => h.ingest(&r),
                    Event::Gap {
                        stream,
                        epoch,
                        from,
                        to,
                        reason,
                    } => h.gap(&stream, &epoch, from, to, &reason),
                    Event::Disconnected { stream, reason } => h.disconnected(&stream, &reason),
                }
            } else {
                break;
            }
        }
        let _ = data_stop.send(true);
    });
    let control_hub = hub.clone();
    let control_client = client.clone();
    let control_stop = stop_tx.clone();
    let control_url = url.clone();
    let control_task = tokio::spawn(async move {
        while let Some(call) = controls.recv().await {
            if call.method == "shutdown" {
                let _ = control_client
                    .reply_control(call.call_id, json!({"stopping":true}), None)
                    .await;
                let _ = control_stop.send(true);
                break;
            }
            let result = if call.method == "session.select" {
                let id = call.args.get("id").and_then(Value::as_str).unwrap_or("");
                lock(&control_hub).and_then(|hub|hub.snapshot(id)).map(|snapshot|json!({"id":id,"revision":snapshot.revision,"url":format!("{control_url}/?session={id}"),"scope":"open this URL; other browser views are unchanged"}))
            } else {
                log_plot::control(&control_hub, &call.method, call.args).await
            };
            let (value, error) = match result {
                Ok(value) => (value, None),
                Err(e) => (
                    Value::Null,
                    Some(Fault {
                        code: if e.to_string().starts_with("revision_conflict") {
                            "revision_conflict"
                        } else {
                            "invalid_control"
                        }
                        .into(),
                        message: e.to_string(),
                    }),
                ),
            };
            if let Err(e) = control_client
                .reply_control(call.call_id, value, error)
                .await
            {
                eprintln!("WebUI control reply: {e}");
            }
        }
        let _ = control_stop.send(true);
    });
    let state = App {
        hub: hub.clone(),
        connections: Arc::new(Semaphore::new(16)),
        exports: Arc::new(Semaphore::new(2)),
        stop: stop_tx.subscribe(),
    };
    let app = Router::new()
        .route("/", get(index))
        .route("/assets/echarts.min.js", get(echarts))
        .route("/assets/app.js", get(app_js))
        .route("/assets/style.css", get(style))
        .route("/api/sessions", get(sessions))
        .route("/api/sessions/{id}", get(snapshot))
        .route("/api/sessions/{id}/patch", post(patch))
        .route("/api/sessions/{id}/events", get(events_sse))
        .route("/api/sessions/{id}/image/{format}", get(image))
        .route("/health", get(|| async { Json(json!({"ready":true})) }))
        .with_state(state);
    let session_ids = lock(&hub)?.ids();
    client.request("report",json!({"kind":"webui","ready":true,"url":url,"sessions":session_ids,"offline_assets":true})).await?;
    eprintln!("WebUI ready: {url}");
    axum::serve(listener,app).with_graceful_shutdown(async move{tokio::select!{_=async{while !*stop_rx.borrow(){if stop_rx.changed().await.is_err(){break;}}}=>{},_=shutdown_signal()=>{}}}).await?;
    data_task.abort();
    control_task.abort();
    Ok(())
}
async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_err() {
        // A detached Windows process can lack a console signal source.
        // The supervisor control channel remains the shutdown mechanism.
        std::future::pending::<()>().await;
    }
}
async fn index() -> impl IntoResponse {
    ([(header::CACHE_CONTROL,"no-store"),(header::CONTENT_SECURITY_POLICY,"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'")],Html(include_str!("../assets/index.html")))
}
async fn echarts() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_str!("../assets/echarts.min.js"),
    )
}
async fn app_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/app.js"),
    )
}
async fn style() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        include_str!("../assets/style.css"),
    )
}
async fn sessions(State(app): State<App>) -> ApiResult<Json<Value>> {
    Ok(Json(lock(&app.hub)?.summaries()))
}
async fn snapshot(State(app): State<App>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(serde_json::to_value(lock(&app.hub)?.snapshot(&id)?)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    revision: u64,
    patch: Value,
}
async fn patch(
    State(app): State<App>,
    Path(id): Path<String>,
    Json(body): Json<Patch>,
) -> ApiResult<Json<Value>> {
    Ok(Json(lock(&app.hub)?.patch(
        &id,
        body.revision,
        &body.patch,
    )?))
}
async fn events_sse(
    State(app): State<App>,
    Path(id): Path<String>,
) -> ApiResult<Sse<impl futures_core::Stream<Item = std::result::Result<SseEvent, Infallible>>>> {
    let permit = app
        .connections
        .clone()
        .try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("16 live-view connections already open"))?;
    lock(&app.hub)?.snapshot(&id)?;
    let stream = async_stream::stream! {
        let _permit=permit;let mut previous=None;let mut stop=app.stop.clone();
        loop {
            if *stop.borrow(){break;}
            let snapshot={match lock(&app.hub).and_then(|h|h.snapshot(&id)){Ok(s)=>s,Err(e)=>{yield Ok(SseEvent::default().event("error").data(e.to_string()));break;}}};
            let delay=Duration::from_millis(snapshot.refresh_ms);
            if previous!=Some(snapshot.generation){previous=Some(snapshot.generation);
                match serde_json::to_string(&snapshot){Ok(data)=>yield Ok(SseEvent::default().event("snapshot").id(snapshot.generation.to_string()).data(data)),Err(e)=>{yield Ok(SseEvent::default().event("error").data(e.to_string()));break;}}
            }
            tokio::select!{_=tokio::time::sleep(delay)=>{},_=stop.changed()=>{}}
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}
#[derive(Deserialize)]
struct ImageQuery {
    revision: Option<u64>,
}
async fn image(
    State(app): State<App>,
    Path((id, format)): Path<(String, String)>,
    Query(query): Query<ImageQuery>,
) -> ApiResult<Response> {
    let permit = app
        .exports
        .clone()
        .try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("two exports already running"))?;
    let snapshot = lock(&app.hub)?.snapshot(&id)?;
    if let Some(revision) = query.revision {
        if revision != snapshot.revision {
            return Err(anyhow::anyhow!(
                "revision_conflict: current revision is {}",
                snapshot.revision
            )
            .into());
        }
    }
    if !matches!(format.as_str(), "png" | "svg") {
        return Err(anyhow::anyhow!("format must be png or svg").into());
    }
    let mime = if format == "png" {
        "image/png"
    } else {
        "image/svg+xml"
    };
    let filename = format!("attachment; filename=\"{id}.{}\"", format);
    let bytes =
        tokio::task::spawn_blocking(move || log_plot::encode(&snapshot, &format, 1200, 700))
            .await
            .map_err(anyhow::Error::from)??;
    let body = axum::body::Body::from_stream(
        async_stream::stream! {let _permit=permit;yield Ok::<_,Infallible>(bytes);},
    );
    Ok((
        [
            (header::CONTENT_TYPE, mime),
            (header::CONTENT_DISPOSITION, &filename),
            (header::CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response())
}
