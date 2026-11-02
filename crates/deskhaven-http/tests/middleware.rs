//! Middleware stack behaviour (06-security §6, 02-architecture §6).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderValue, Request, Response, StatusCode, header};
use axum::routing::{get, post};
use deskhaven_http::{HttpConfig, X_REQUEST_ID, apply_middleware};
use tower::ServiceExt;

const SLOW: Duration = Duration::from_secs(30);

async fn boom() -> &'static str {
    panic!("handler exploded with internal detail")
}

fn app(cfg: &HttpConfig) -> Router {
    let routes = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .route("/panic", get(boom))
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(SLOW).await;
                "late"
            }),
        )
        .route(
            "/echo",
            post(|body: Bytes| async move { body.len().to_string() }),
        )
        .route(
            "/own-header",
            get(|| async { ([(header::X_FRAME_OPTIONS, "SAMEORIGIN")], "x") }),
        );
    apply_middleware(routes, cfg)
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

#[tokio::test]
async fn panic_returns_generic_500_and_router_keeps_serving() {
    let app = app(&HttpConfig::default());

    let res = send(app.clone(), get_req("/panic")).await;
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(res.headers().contains_key(X_REQUEST_ID));
    assert!(res.headers().contains_key(header::X_CONTENT_TYPE_OPTIONS));
    let body = body_text(res).await;
    assert_eq!(body, "internal server error");

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
async fn csp_hook_sets_header_when_configured() {
    let cfg = HttpConfig {
        content_security_policy: Some(HeaderValue::from_static("default-src 'none'")),
        ..HttpConfig::default()
    };
    let res = send(app(&cfg), get_req("/ok")).await;
    assert_eq!(
        res.headers()[header::CONTENT_SECURITY_POLICY],
        "default-src 'none'"
    );
}

#[tokio::test]
async fn request_id_is_generated_when_absent() {
    let app = app(&HttpConfig::default());
    let first = send(app.clone(), get_req("/ok")).await;
    let second = send(app, get_req("/ok")).await;
    let a = first.headers()[X_REQUEST_ID].to_str().unwrap().to_owned();
    let b = second.headers()[X_REQUEST_ID].to_str().unwrap().to_owned();
    assert_eq!(a.len(), 36, "uuid: {a}");
    assert_ne!(a, b);
}

#[tokio::test]
async fn valid_incoming_request_id_is_echoed() {
    let req = Request::get("/ok")
        .header(X_REQUEST_ID, "abc-123_x.y:z")
        .body(Body::empty())
        .unwrap();
    let res = send(app(&HttpConfig::default()), req).await;
    assert_eq!(res.headers()[X_REQUEST_ID], "abc-123_x.y:z");
}

#[tokio::test]
async fn invalid_incoming_request_id_is_replaced() {
    let too_long = "a".repeat(65);
    for bad in ["has space", "semi;colon", "", too_long.as_str()] {
        let req = Request::get("/ok")
            .header(X_REQUEST_ID, bad)
            .body(Body::empty())
            .unwrap();
        let res = send(app(&HttpConfig::default()), req).await;
        let id = res.headers()[X_REQUEST_ID].to_str().unwrap();
        assert_ne!(id, bad);
        assert_eq!(id.len(), 36);
    }
}

#[tokio::test]
async fn slow_handler_times_out_with_503() {
    let cfg = HttpConfig {
        request_timeout: Duration::from_millis(20),
        ..HttpConfig::default()
    };
    let res = tokio::time::timeout(Duration::from_secs(5), send(app(&cfg), get_req("/slow")))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(res.headers().contains_key(X_REQUEST_ID));
}

fn limited(limit: usize) -> Router {
    app(&HttpConfig {
        body_limit: limit,
        ..HttpConfig::default()
    })
}

#[tokio::test]
async fn body_within_limit_is_accepted() {
    let req = Request::post("/echo")
        .body(Body::from(vec![0u8; 16]))
        .unwrap();
    let res = send(limited(16), req).await;
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_text(res).await, "16");
}

#[tokio::test]
async fn oversized_content_length_is_413() {
    let req = Request::post("/echo")
        .header(header::CONTENT_LENGTH, "17")
        .body(Body::from(vec![0u8; 17]))
        .unwrap();
    assert_eq!(
        send(limited(16), req).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn oversized_body_without_content_length_is_413() {
    let req = Request::post("/echo")
        .body(Body::from(vec![0u8; 17]))
        .unwrap();
    assert_eq!(
        send(limited(16), req).await.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[tokio::test]
async fn body_limit_overrides_axum_default_of_2mib() {
    let size = 3 * 1024 * 1024;
    let req = Request::post("/echo")
        .body(Body::from(vec![0u8; size]))
        .unwrap();
    let res = send(limited(4 * 1024 * 1024), req).await;
    assert_eq!(res.status(), StatusCode::OK);
}
