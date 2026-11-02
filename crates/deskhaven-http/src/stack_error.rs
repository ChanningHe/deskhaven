//! Errors produced by the HTTP stack itself (unknown route, wrong method, body
//! too large, timeout, panic, ...) rendered in the API's contract shape.
//!
//! Any 4xx/5xx response *without* a JSON content type is treated as
//! stack-generated: its body is replaced by the renderer's (the status and other
//! headers are kept), so framework wording never reaches clients. Handlers that
//! want their own error body must return JSON (both API error types do).
//!
//! The request deadline lives here as well because a timeout has to be rendered:
//! it returns 503, which RustDesk clients retry once through hbbs (03 §1.4).

use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, Response, StatusCode, header};
use axum::middleware::Next;
use axum::response::IntoResponse;

use crate::paths;

/// What went wrong, without any detail worth hiding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackError {
    /// Any other 4xx (e.g. an extractor rejection).
    BadRequest,
    NotFound,
    MethodNotAllowed,
    PayloadTooLarge,
    /// The request deadline expired (503).
    Timeout,
    /// A 503 that is not a timeout (e.g. `/readyz`).
    Unavailable,
    /// Any 5xx other than 503, including panics.
    Internal,
}

impl StackError {
    fn classify(status: StatusCode) -> Self {
        match status {
            StatusCode::NOT_FOUND => Self::NotFound,
            StatusCode::METHOD_NOT_ALLOWED => Self::MethodNotAllowed,
            StatusCode::PAYLOAD_TOO_LARGE => Self::PayloadTooLarge,
            StatusCode::SERVICE_UNAVAILABLE => Self::Unavailable,
            s if s.is_client_error() => Self::BadRequest,
            _ => Self::Internal,
        }
    }

    /// Generic, user-facing text.
    pub const fn message(self) -> &'static str {
        match self {
            Self::BadRequest => "bad request",
            Self::NotFound => "not found",
            Self::MethodNotAllowed => "method not allowed",
            Self::PayloadTooLarge => "payload too large",
            Self::Timeout => "request timed out",
            Self::Unavailable => "service unavailable",
            Self::Internal => crate::INTERNAL_ERROR_BODY,
        }
    }
}

/// Builds the body for a stack error. The status it sets is ignored: the
/// original status is kept (503 for timeouts).
pub type ErrorRenderer = fn(StackError) -> Response<Body>;

/// Plain-text `message()`; the default when no API renderer is plugged in.
pub fn plain_text(err: StackError) -> Response<Body> {
    err.message().into_response()
}

/// Renderers per API. Paths under [`paths::CLIENT_API_PREFIX`] use `client_api`,
/// everything else (admin API, console) uses `admin`.
#[derive(Debug, Clone, Copy)]
pub struct ErrorBodies {
    pub client_api: ErrorRenderer,
    pub admin: ErrorRenderer,
}

impl ErrorBodies {
    /// One renderer for every path (a listener that serves a single API).
    pub const fn uniform(renderer: ErrorRenderer) -> Self {
        Self {
            client_api: renderer,
            admin: renderer,
        }
    }

    fn select(&self, path: &str) -> ErrorRenderer {
        if paths::is_client_api(path) {
            self.client_api
        } else {
            self.admin
        }
    }
}

impl Default for ErrorBodies {
    fn default() -> Self {
        Self::uniform(plain_text)
    }
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| {
            let essence = ct.split(';').next().unwrap_or_default().trim();
            essence.eq_ignore_ascii_case("application/json") || essence.ends_with("+json")
        })
}

/// `res` with its body replaced by `render(err)`, keeping status and headers.
fn rerender(res: Response<Body>, render: ErrorRenderer, err: StackError) -> Response<Body> {
    let (mut parts, _) = res.into_parts();
    let (rendered, body) = render(err).into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    match rendered.headers.get(header::CONTENT_TYPE) {
        Some(ct) => parts.headers.insert(header::CONTENT_TYPE, ct.clone()),
        None => parts.headers.remove(header::CONTENT_TYPE),
    };
    Response::from_parts(parts, body)
}

pub(crate) async fn render_stack_errors(
    bodies: ErrorBodies,
    deadline: Duration,
    req: Request,
    next: Next,
) -> Response<Body> {
    let render = bodies.select(req.uri().path());
    let Ok(res) = tokio::time::timeout(deadline, next.run(req)).await else {
        let timed_out = StatusCode::SERVICE_UNAVAILABLE.into_response();
        return rerender(timed_out, render, StackError::Timeout);
    };
    let status = res.status();
    let is_error = status.is_client_error() || status.is_server_error();
    if !is_error || is_json(res.headers()) {
        return res;
    }
    rerender(res, render, StackError::classify(status))
}
