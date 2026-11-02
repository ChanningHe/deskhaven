//! The request handler: exact asset, SPA fallback or 404.

use axum::Router;
use axum::extract::Request;
use axum::http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

use crate::accept::accepts_html;
use crate::file::File;
use crate::not_built::NOT_BUILT_HTML;
use crate::request_path::{Target, classify};

const INDEX: &str = "index.html";

/// Every path is handled by the fallback so the router can be merged into the
/// admin router (which must not define a fallback of its own).
pub(crate) fn router<E: RustEmbed + 'static>() -> Router {
    Router::new().fallback(handle::<E>)
}

async fn handle<E: RustEmbed>(req: Request) -> Response {
    let read = matches!(*req.method(), Method::GET | Method::HEAD);
    let target = classify(req.uri().path());
    // API paths belong to the API's own 404 whatever the method.
    let Target::Path { key, has_extension } = target else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !read {
        return (StatusCode::METHOD_NOT_ALLOWED, [(ALLOW, "GET, HEAD")]).into_response();
    }
    let file = if key.is_empty() {
        None
    } else {
        E::get(&key).map(|embedded| File::new(&key, embedded))
    };
    if let Some(file) = file {
        return file.respond(req.headers());
    }
    // Missing files and non-HTML clients get a real 404, never the SPA.
    if has_extension || !accepts_html(req.headers()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match E::get(INDEX) {
        Some(embedded) => File::new(INDEX, embedded).respond(req.headers()),
        None => not_built(),
    }
}

/// 503: the binary was built without `web/dist`.
fn not_built() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [
            (
                CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        NOT_BUILT_HTML,
    )
        .into_response()
}
