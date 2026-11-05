//! The middleware stack every listener gets (02-architecture §6, 06-security §6).
//!
//! Order, outermost first:
//! request id -> trace span -> probe marker -> security headers -> API `Cache-Control` ->
//! stack-error rendering + request deadline -> catch-panic -> trusted proxies
//! extension -> body limit -> router.
//! Everything inside the security-header layer (panics, timeouts, 413s, 404s)
//! still carries the request id and the security headers. No CORS layer: the
//! API is same-origin only.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderName, HeaderValue, Response, StatusCode, header};
use axum::middleware::{Next, from_fn};
use axum::response::IntoResponse;
use axum::{Extension, Router};
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing::Span;
use tracing::field::Empty;

use crate::config::HttpConfig;
use crate::paths;
use crate::request_id::{X_REQUEST_ID, propagate};
use crate::stack_error::{ErrorBodies, render_stack_errors};

/// Body of every 500 produced by this crate. Deliberately says nothing.
pub const INTERNAL_ERROR_BODY: &str = "internal server error";

/// Wrap `router` (routes and fallback) in the full middleware stack.
pub fn apply_middleware<S>(router: Router<S>, cfg: &HttpConfig, errors: ErrorBodies) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let trusted = cfg.trusted_proxies.clone();
    let headers = security_headers(cfg);
    let deadline = cfg.request_timeout;
    router.layer(
        ServiceBuilder::new()
            .layer(from_fn(move |req: Request, next: Next| {
                propagate(trusted.clone(), req, next)
            }))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(make_span)
                    .on_response(on_response),
            )
            .layer(from_fn(mark_probes))
            .layer(from_fn(move |req: Request, next: Next| {
                set_security_headers(headers.clone(), req, next)
            }))
            .layer(from_fn(api_no_store))
            .layer(from_fn(move |req: Request, next: Next| {
                render_stack_errors(errors, deadline, req, next)
            }))
            .layer(CatchPanicLayer::custom(panic_response))
            .layer(Extension(cfg.trusted_proxies.clone()))
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(cfg.body_limit)),
    )
}

const NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");
const SAME_ORIGIN: HeaderValue = HeaderValue::from_static("same-origin");
const DENY: HeaderValue = HeaderValue::from_static("DENY");
const NO_STORE: HeaderValue = HeaderValue::from_static("no-store");

type HeaderList = Arc<[(HeaderName, HeaderValue)]>;

fn security_headers(cfg: &HttpConfig) -> HeaderList {
    let optional = [
        (
            header::CONTENT_SECURITY_POLICY,
            &cfg.content_security_policy,
        ),
        (
            header::STRICT_TRANSPORT_SECURITY,
            &cfg.strict_transport_security,
        ),
    ];
    [
        (header::X_CONTENT_TYPE_OPTIONS, NOSNIFF),
        (header::REFERRER_POLICY, SAME_ORIGIN),
        (header::X_FRAME_OPTIONS, DENY),
    ]
    .into_iter()
    .chain(
        optional
            .into_iter()
            .filter_map(|(name, value)| value.clone().map(|v| (name, v))),
    )
    .collect()
}

/// Each header is set only when the handler did not set it.
async fn set_security_headers(headers: HeaderList, req: Request, next: Next) -> Response<Body> {
    let mut res = next.run(req).await;
    for (name, value) in headers.iter() {
        if !res.headers().contains_key(name) {
            res.headers_mut().insert(name.clone(), value.clone());
        }
    }
    res
}

/// API responses are per-user and must not be cached; static assets keep theirs.
async fn api_no_store(req: Request, next: Next) -> Response<Body> {
    let is_api = paths::is_api(req.uri().path());
    let mut res = next.run(req).await;
    if is_api && !res.headers().contains_key(header::CACHE_CONTROL) {
        res.headers_mut().insert(header::CACHE_CONTROL, NO_STORE);
    }
    res
}

/// Method, path (never the query string), request id. Status and latency are
/// recorded when the response is ready.
fn make_span<B>(req: &Request<B>) -> Span {
    let request_id = req
        .headers()
        .get(X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    tracing::info_span!(
        "http_request",
        method = %req.method(),
        // Display, not Debug: a URI path and a validated request id are plain ASCII.
        path = %req.uri().path(),
        request_id = %request_id,
        status = Empty,
        latency_ms = Empty,
    )
}

/// Marks responses to health probes, for [`on_response`] (which only sees
/// the response).
#[derive(Clone, Copy)]
struct Probe;

async fn mark_probes(req: Request, next: Next) -> Response<Body> {
    let is_probe = paths::is_probe(req.uri().path());
    let mut res = next.run(req).await;
    if is_probe {
        res.extensions_mut().insert(Probe);
    }
    res
}

fn on_response<B>(res: &Response<B>, latency: Duration, span: &Span) {
    span.record("status", res.status().as_u16());
    span.record("latency_ms", latency.as_millis());
    if res.extensions().get::<Probe>().is_some() {
        tracing::debug!("request finished");
    } else {
        tracing::info!("request finished");
    }
}

/// The panic message is logged (it may contain values; logs are server-side),
/// never returned; the body is re-rendered as [`crate::StackError::Internal`].
fn panic_response(err: Box<dyn Any + Send + 'static>) -> Response<Body> {
    let detail = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| err.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload");
    tracing::error!(panic = detail, "handler panicked");
    (StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR_BODY).into_response()
}
