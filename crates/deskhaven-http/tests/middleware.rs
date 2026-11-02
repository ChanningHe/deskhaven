//! Middleware stack behaviour (06-security §6, 02-architecture §6).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderValue, Request, Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use deskhaven_http::{
    ErrorBodies, HttpConfig, StackError, TrustedProxies, X_REQUEST_ID, apply_middleware,
};
use tower::ServiceExt;

const SLOW: Duration = Duration::from_secs(30);

async fn boom() -> &'static str {
    panic!("handler exploded with internal detail")
}

/// Error-producing routes, mounted at the root, under `/api` and under
/// `/admin/api` (re-rendering only applies to the API prefixes).
fn fixtures(prefix: &str) -> Router {
    let at = |path: &str| format!("{prefix}{path}");
    Router::new()
        .route(&at("/panic"), get(boom))
        .route(
            &at("/slow"),
            get(|| async {
                tokio::time::sleep(SLOW).await;
                "late"
            }),
        )
        .route(
            &at("/echo"),
            post(|body: Bytes| async move { body.len().to_string() }),
        )
        .route(
            &at("/plain-400"),
            get(|| async { (StatusCode::BAD_REQUEST, "axum wording: detail") }),
        )
        .route(&at("/json-400"), get(|| async { json_error() }))
        .route(
            &at("/not-ready"),
            get(|| async { (StatusCode::SERVICE_UNAVAILABLE, "not ready") }),
        )
}

fn routes() -> Router {
    Router::new()
        .route("/ok", get(|| async { "ok" }))
        .route(
            "/own-header",
            get(|| async { ([(header::X_FRAME_OPTIONS, "SAMEORIGIN")], "x") }),
        )
        .route("/api/thing", get(|| async { "api" }))
        .route("/admin/api/thing", get(|| async { "admin" }))
        .route(
            "/api/cached",
            get(|| async { ([(header::CACHE_CONTROL, "max-age=5")], "c") }),
        )
        .route("/assets/app.js", get(|| async { "js" }))
        .merge(fixtures(""))
        .merge(fixtures("/api"))
        .merge(fixtures("/admin/api"))
}

fn json_error() -> Response<Body> {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"handler"}"#,
    )
        .into_response()
}

fn app(cfg: &HttpConfig) -> Router {
    apply_middleware(routes(), cfg, ErrorBodies::default())
}

/// Renders a recognisable body with a wrong status, to prove the stack keeps
/// the original status.
fn client_render(e: StackError) -> Response<Body> {
    (StatusCode::IM_A_TEAPOT, format!("client:{e:?}")).into_response()
}

fn admin_render(e: StackError) -> Response<Body> {
    (StatusCode::IM_A_TEAPOT, format!("admin:{e:?}")).into_response()
}

fn tagging() -> ErrorBodies {
    ErrorBodies {
        client_api: client_render,
        admin: admin_render,
    }
}

fn tagged() -> Router {
    apply_middleware(routes(), &HttpConfig::default(), tagging())
}

async fn send(app: Router, req: Request<Body>) -> Response<Body> {
    app.oneshot(req).await.unwrap()
}

fn get_req(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

async fn body_text(res: Response<Body>) -> String {
    let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn status_and_body(app: Router, req: Request<Body>) -> (StatusCode, String) {
    let res = send(app, req).await;
    (res.status(), body_text(res).await)
}

// --- panics, headers ------------------------------------------------------------

#[tokio::test]
async fn panic_returns_generic_500_and_router_keeps_serving() {
    let app = app(&HttpConfig::default());

    let res = send(app.clone(), get_req("/panic")).await;
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(res.headers().contains_key(X_REQUEST_ID));
    assert!(res.headers().contains_key(header::X_CONTENT_TYPE_OPTIONS));
    assert_eq!(body_text(res).await, "internal server error");

    let res = send(app, get_req("/ok")).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn security_headers_are_set() {
    let res = send(app(&HttpConfig::default()), get_req("/ok")).await;
    let h = res.headers();
    assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(h[header::REFERRER_POLICY], "same-origin");
    assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
    assert!(!h.contains_key(header::CONTENT_SECURITY_POLICY));
    assert!(!h.contains_key(header::STRICT_TRANSPORT_SECURITY));
    assert!(!h.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[tokio::test]
async fn security_headers_also_on_404() {
    let res = send(app(&HttpConfig::default()), get_req("/missing")).await;
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    assert_eq!(res.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(res.headers().contains_key(X_REQUEST_ID));
}

#[tokio::test]
async fn handler_set_security_header_is_not_overridden() {
    let res = send(app(&HttpConfig::default()), get_req("/own-header")).await;
    assert_eq!(res.headers()[header::X_FRAME_OPTIONS], "SAMEORIGIN");
}

#[tokio::test]
async fn csp_and_hsts_hooks_set_headers_when_configured() {
    let cfg = HttpConfig {
        content_security_policy: Some(HeaderValue::from_static("default-src 'none'")),
        strict_transport_security: Some(HeaderValue::from_static("max-age=31536000")),
        ..HttpConfig::default()
    };
    let res = send(app(&cfg), get_req("/ok")).await;
    assert_eq!(
        res.headers()[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'"
    );
    assert_eq!(
        res.headers()[header::STRICT_TRANSPORT_SECURITY],
        "max-age=31536000"
    );
}

#[tokio::test]
async fn api_responses_are_not_cacheable() {
    for path in ["/api/thing", "/admin/api/thing", "/api/missing"] {
        let res = send(app(&HttpConfig::default()), get_req(path)).await;
        assert_eq!(res.headers()[header::CACHE_CONTROL], "no-store", "{path}");
    }
}

#[tokio::test]
async fn non_api_responses_keep_their_cache_policy() {
    for path in ["/ok", "/assets/app.js", "/apix", "/admin/apix"] {
        let res = send(app(&HttpConfig::default()), get_req(path)).await;
        assert!(!res.headers().contains_key(header::CACHE_CONTROL), "{path}");
    }
    let res = send(app(&HttpConfig::default()), get_req("/api/cached")).await;
    assert_eq!(res.headers()[header::CACHE_CONTROL], "max-age=5");
}

// --- request id -----------------------------------------------------------------

fn id_request(id: &str) -> Request<Body> {
    Request::get("/ok")
        .header(X_REQUEST_ID, id)
        .body(Body::empty())
        .unwrap()
}

fn behind_proxy() -> Router {
    let cfg = HttpConfig {
        trusted_proxies: TrustedProxies::new(["10.0.0.0/8".parse().unwrap()]),
        ..HttpConfig::default()
    };
    app(&cfg)
}

fn from_peer(app: Router, peer: &str) -> Router {
    app.layer(MockConnectInfo(peer.parse::<SocketAddr>().unwrap()))
}

fn response_id(res: &Response<Body>) -> String {
    res.headers()[X_REQUEST_ID].to_str().unwrap().to_owned()
}

#[tokio::test]
async fn request_id_is_generated_when_absent() {
    let app = app(&HttpConfig::default());
    let a = response_id(&send(app.clone(), get_req("/ok")).await);
    let b = response_id(&send(app, get_req("/ok")).await);
    assert_eq!(a.len(), 36, "uuid: {a}");
    assert_ne!(a, b);
}

#[tokio::test]
async fn incoming_request_id_from_trusted_proxy_is_echoed() {
    let app = from_peer(behind_proxy(), "10.0.0.1:4000");
    let res = send(app, id_request("abc-123_x.y:z")).await;
    assert_eq!(response_id(&res), "abc-123_x.y:z");
}

#[tokio::test]
async fn incoming_request_id_from_untrusted_peer_is_replaced() {
    for app in [
        from_peer(behind_proxy(), "203.0.113.5:4000"),
        behind_proxy(),
    ] {
        let id = response_id(&send(app, id_request("client-chosen")).await);
        assert_ne!(id, "client-chosen");
        assert_eq!(id.len(), 36);
    }
}

#[tokio::test]
async fn invalid_request_id_from_trusted_proxy_is_replaced() {
    let too_long = "a".repeat(65);
    for bad in ["has space", "semi;colon", "", too_long.as_str()] {
        let app = from_peer(behind_proxy(), "10.0.0.1:4000");
        let id = response_id(&send(app, id_request(bad)).await);
        assert_ne!(id, bad);
        assert_eq!(id.len(), 36);
    }
}

// --- timeout and body limit ---------------------------------------------------------

fn timing_out() -> HttpConfig {
    HttpConfig {
        request_timeout: Duration::from_millis(20),
        ..HttpConfig::default()
    }
}

async fn within_deadline(app: Router, path: &str) -> Response<Body> {
    tokio::time::timeout(Duration::from_secs(5), send(app, get_req(path)))
        .await
        .unwrap()
}

#[tokio::test]
async fn slow_handler_times_out_with_503_and_plain_body() {
    for path in ["/slow", "/api/slow"] {
        let res = within_deadline(app(&timing_out()), path).await;
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert!(res.headers().contains_key(X_REQUEST_ID));
        assert_eq!(body_text(res).await, "request timed out", "{path}");
    }
}

#[tokio::test]
async fn api_timeout_uses_the_api_renderer() {
    let tagged_timeout = apply_middleware(routes(), &timing_out(), tagging());
    let res = within_deadline(tagged_timeout, "/api/slow").await;
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_text(res).await, "client:Timeout");
}

fn limited(limit: usize) -> Router {
    app(&HttpConfig {
        body_limit: limit,
        ..HttpConfig::default()
    })
}

fn post_bytes(path: &str, len: usize) -> Request<Body> {
    Request::post(path)
        .body(Body::from(vec![0u8; len]))
        .unwrap()
}

#[tokio::test]
async fn body_within_limit_is_accepted() {
    let res = send(limited(16), post_bytes("/api/echo", 16)).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_text(res).await, "16");
}

#[tokio::test]
async fn oversized_content_length_is_413() {
    let req = Request::post("/api/echo")
        .header(header::CONTENT_LENGTH, "17")
        .body(Body::from(vec![0u8; 17]))
        .unwrap();
    let got = status_and_body(limited(16), req).await;
    assert_eq!(
        got,
        (StatusCode::PAYLOAD_TOO_LARGE, "payload too large".into())
    );
}

#[tokio::test]
async fn oversized_body_without_content_length_is_413() {
    let got = status_and_body(limited(16), post_bytes("/api/echo", 17)).await;
    assert_eq!(
        got,
        (StatusCode::PAYLOAD_TOO_LARGE, "payload too large".into())
    );
}

#[tokio::test]
async fn oversized_body_outside_api_is_413_but_not_rerendered() {
    let app = apply_middleware(
        routes(),
        &HttpConfig {
            body_limit: 4,
            ..HttpConfig::default()
        },
        tagging(),
    );
    let (status, body) = status_and_body(app, post_bytes("/echo", 5)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!body.starts_with("admin:"), "{body}");
}

#[tokio::test]
async fn body_limit_overrides_axum_default_of_2mib() {
    let req = post_bytes("/api/echo", 3 * 1024 * 1024);
    let res = send(limited(4 * 1024 * 1024), req).await;
    assert_eq!(res.status(), StatusCode::OK);
}

// --- error bodies (review M1) -----------------------------------------------------------

#[tokio::test]
async fn api_stack_errors_are_rendered_by_prefix_keeping_status() {
    let cases = [
        ("/api/missing", StatusCode::NOT_FOUND, "client:NotFound"),
        (
            "/api/plain-400",
            StatusCode::BAD_REQUEST,
            "client:BadRequest",
        ),
        (
            "/admin/api/missing",
            StatusCode::NOT_FOUND,
            "admin:NotFound",
        ),
        (
            "/admin/api/panic",
            StatusCode::INTERNAL_SERVER_ERROR,
            "admin:Internal",
        ),
        (
            "/admin/api/plain-400",
            StatusCode::BAD_REQUEST,
            "admin:BadRequest",
        ),
        (
            "/admin/api/not-ready",
            StatusCode::SERVICE_UNAVAILABLE,
            "admin:Unavailable",
        ),
    ];
    for (path, status, body) in cases {
        let got = status_and_body(tagged(), get_req(path)).await;
        assert_eq!(got, (status, body.to_owned()), "{path}");
    }
}

/// Future SPA routes and static assets keep whatever the stack produced.
#[tokio::test]
async fn non_api_errors_pass_through_untouched() {
    let cases = [
        ("/elsewhere", StatusCode::NOT_FOUND, ""),
        ("/apix", StatusCode::NOT_FOUND, ""),
        ("/admin/apix/x", StatusCode::NOT_FOUND, ""),
        (
            "/plain-400",
            StatusCode::BAD_REQUEST,
            "axum wording: detail",
        ),
        ("/not-ready", StatusCode::SERVICE_UNAVAILABLE, "not ready"),
        (
            "/panic",
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error",
        ),
    ];
    for (path, status, body) in cases {
        let got = status_and_body(tagged(), get_req(path)).await;
        assert_eq!(got, (status, body.to_owned()), "{path}");
    }
}

#[tokio::test]
async fn method_not_allowed_is_rendered_and_keeps_allow_header() {
    let req = Request::delete("/api/thing").body(Body::empty()).unwrap();
    let res = send(tagged(), req).await;
    assert_eq!(res.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(res.headers().contains_key(header::ALLOW));
    assert_eq!(body_text(res).await, "client:MethodNotAllowed");
}

#[tokio::test]
async fn payload_too_large_uses_the_path_renderer() {
    let app = apply_middleware(
        routes(),
        &HttpConfig {
            body_limit: 4,
            ..HttpConfig::default()
        },
        tagging(),
    );
    let got = status_and_body(app, post_bytes("/api/echo", 5)).await;
    assert_eq!(
        got,
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "client:PayloadTooLarge".into()
        )
    );
}

#[tokio::test]
async fn json_and_success_responses_are_untouched() {
    let res = send(tagged(), get_req("/admin/api/json-400")).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_text(res).await, r#"{"error":"handler"}"#);
    let got = status_and_body(tagged(), get_req("/api/thing")).await;
    assert_eq!(got, (StatusCode::OK, "api".into()));
}

#[tokio::test]
async fn axum_rejection_wording_never_leaks_from_the_api() {
    for path in ["/api/plain-400", "/admin/api/plain-400"] {
        let (status, body) = status_and_body(tagged(), get_req(path)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(!body.contains("axum wording"), "{path}: {body}");
    }
}
