//! Serves the built Svelte UI (`web/ui/dist`) in the `full` build. Embedded
//! into the binary at compile time so the router still ships as one file.

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "web/ui/dist/"]
struct Assets;

/// `/` serves `index.html` (its own open route, like `app.Get("/", ...)` in
/// `cmd/ui.go`); `/ui/<path>` serves the built assets it references (its own
/// open route, like `app.Use("/ui", filesystem...)`). Everything else falls
/// through to a plain 404, matching the Go build's behaviour of not treating
/// unknown top-level paths as SPA routes.
pub async fn serve_ui(uri: Uri) -> Response {
    let path = uri.path();
    let asset_path = if path == "/" {
        "index.html"
    } else if let Some(rest) = path.strip_prefix("/ui/") {
        rest
    } else {
        return not_found();
    };

    match Assets::get(asset_path) {
        Some(file) => respond(asset_path, file.data.into_owned()),
        None => not_found(),
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn respond(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(data))
        .unwrap()
}
