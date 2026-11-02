//! The middleware stack every listener gets (02-architecture §6, 06-security §6).
//!
//! Order, outermost first:
//! request id -> trace span -> security headers -> catch-panic -> trusted proxies
//! extension -> timeout -> body limit -> router.
//! Everything inside the security-header layer (panics, timeouts, 413s, 404s)
//! still carries the request id and the security headers. No CORS layer: the
//! API is same-origin only.

use std::any::Any;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderValue, Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::{Extension, Router};
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::Span;
use tracing::field::Empty;

use crate::real_ip::TrustedProxies;
use crate::request_id::{X_REQUEST_ID, propagate};

pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_BODY_LIMIT: usize = 2 * 1024 * 1024;

/// Body of every 500 produced by this crate. Deliberately says nothing.
pub const INTERNAL_ERROR_BODY: &str = "internal server error";

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub trusted_proxies: TrustedProxies,
    /// Whole-request deadline; exceeded -> 503.
    pub request_timeout: Duration,
    /// Max request body in bytes; exceeded -> 413. Replaces axum's 2 MiB default.
    pub body_limit: usize,
    /// Hook for the web crate: set as `Content-Security-Policy` when the
    /// handler did not set one itself. `None` = no CSP header.
    pub content_security_policy: Option<HeaderValue>,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            trusted_proxies: TrustedProxies::default(),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            body_limit: DEFAULT_BODY_LIMIT,
            content_security_policy: None,
        }
    }
}

/// Wrap `router` (routes and fallback) in the full middleware stack.
pub fn apply_middleware<S>(router: Router<S>, cfg: &HttpConfig) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let security_headers = ServiceBuilder::new()
        .layer(header_if_absent(
            header::X_CONTENT_TYPE_OPTIONS,
            Some(NOSNIFF),
        ))
        .layer(header_if_absent(header::REFERRER_POLICY, Some(SAME_ORIGIN)))
        .layer(header_if_absent(header::X_FRAME_OPTIONS, Some(DENY)))
        .layer(header_if_absent(
            header::CONTENT_SECURITY_POLICY,
            cfg.content_security_policy.clone(),
        ));
    router.layer(
        ServiceBuilder::new()
            .layer(axum::middleware::from_fn(propagate))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(make_span)
                    .on_response(on_response),
            )
            .layer(security_headers)
            .layer(CatchPanicLayer::custom(panic_response))
            .layer(Extension(cfg.trusted_proxies.clone()))
            .layer(TimeoutLayer::with_status_code(
                StatusCode::SERVICE_UNAVAILABLE,
                cfg.request_timeout,
            ))
            .layer(DefaultBodyLimit::disable())
            .layer(RequestBodyLimitLayer::new(cfg.body_limit)),
    )
}

const NOSNIFF: HeaderValue = HeaderValue::from_static("nosniff");
const SAME_ORIGIN: HeaderValue = HeaderValue::from_static("same-origin");
const DENY: HeaderValue = HeaderValue::from_static("DENY");

fn header_if_absent(
    name: header::HeaderName,
    value: Option<HeaderValue>,
) -> SetResponseHeaderLayer<Option<HeaderValue>> {
    SetResponseHeaderLayer::if_not_present(name, value)
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

fn on_response<B>(res: &Response<B>, latency: Duration, span: &Span) {
    span.record("status", res.status().as_u16());
    span.record("latency_ms", latency.as_millis());
    tracing::info!("request finished");
}

fn panic_response(err: Box<dyn Any + Send + 'static>) -> Response<Body> {
    let detail = err
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| err.downcast_ref::<&str>().copied())
        .unwrap_or("non-string panic payload");
    tracing::error!(panic = detail, "handler panicked");
    (StatusCode::INTERNAL_SERVER_ERROR, INTERNAL_ERROR_BODY).into_response()
}
