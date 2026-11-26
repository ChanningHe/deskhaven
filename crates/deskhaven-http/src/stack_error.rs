//! Under the API prefixes, any 4xx/5xx response without a JSON content type is treated as
//! stack-generated and its body replaced by the renderer's (status and headers kept), so
//! framework wording never reaches API clients. The deadline lives here because a timeout
//! must be rendered: 503, which RustDesk clients retry once.

use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, Response, StatusCode, header};
use axum::middleware::Next;
use axum::response::IntoResponse;

use crate::paths;

/// Carries no detail worth hiding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackError {
    /// Any other 4xx (e.g. an extractor rejection).
    BadRequest,
    NotFound,
    MethodNotAllowed,
    PayloadTooLarge,
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

/// The status it sets is ignored: the original status is kept.
pub type ErrorRenderer = fn(StackError) -> Response<Body>;

pub fn plain_text(err: StackError) -> Response<Body> {
    err.message().into_response()
}

#[derive(Debug, Clone, Copy)]
pub struct ErrorBodies {
    pub client_api: ErrorRenderer,
    pub admin: ErrorRenderer,
}

impl ErrorBodies {
    pub const fn uniform(renderer: ErrorRenderer) -> Self {
        Self {
            client_api: renderer,
            admin: renderer,
        }
    }

    fn select(&self, path: &str) -> Option<ErrorRenderer> {
        if paths::is_client_api(path) {
            Some(self.client_api)
        } else if paths::is_admin_api(path) {
            Some(self.admin)
        } else {
            None
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
        return rerender(timed_out, render.unwrap_or(plain_text), StackError::Timeout);
    };
    let Some(render) = render else {
        return res;
    };
    let status = res.status();
    let is_error = status.is_client_error() || status.is_server_error();
    if !is_error || is_json(res.headers()) {
        return res;
    }
    rerender(res, render, StackError::classify(status))
}
