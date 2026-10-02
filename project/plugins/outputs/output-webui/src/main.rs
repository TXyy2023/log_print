use anyhow::Result;
use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use include_dir::{include_dir, Dir};
static ASSETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets");
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
    log_view::server::serve(
        "webui",
        Router::new()
            .route("/", get(index))
            .route("/{*path}", get(asset)),
    )
    .await
}
