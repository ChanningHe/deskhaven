//! Errors produced by the HTTP stack itself use the client contract shape.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::routing::get;
use deskhaven_client_api::render_stack_error;
use deskhaven_http::{ErrorBodies, HttpConfig, StackError, apply_middleware};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn call(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
    let body = to_bytes(res.into_body(), 4096).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

fn app() -> Router {
    let routes = Router::new().route(
        "/api/slow",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            "late"
        }),
    );
    let cfg = HttpConfig {
        request_timeout: Duration::from_millis(20),
        ..HttpConfig::default()
    };
    apply_middleware(routes, &cfg, ErrorBodies::uniform(render_stack_error))
}

#[tokio::test]
async fn renderer_uses_the_contract_shape() {
    let cases = [
        (StackError::NotFound, "not found"),
        (StackError::PayloadTooLarge, "payload too large"),
        (StackError::Timeout, "request timed out"),
        (StackError::Internal, "internal server error"),
    ];
    for (err, message) in cases {
        let res = render_stack_error(err);
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value, json!({ "error": message }));
    }
}

#[tokio::test]
async fn unknown_route_is_a_json_404() {
    let req = Request::get("/api/missing").body(Body::empty()).unwrap();
    let got = call(app(), req).await;
    assert_eq!(
        got,
        (StatusCode::NOT_FOUND, json!({ "error": "not found" }))
    );
}

#[tokio::test]
async fn timeout_is_a_json_503() {
    let req = Request::get("/api/slow").body(Body::empty()).unwrap();
    let got = call(app(), req).await;
    assert_eq!(
        got,
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": "request timed out" })
        )
    );
}
