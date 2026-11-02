//! Errors produced by the HTTP stack itself use the admin envelope.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::routing::post;
use deskhaven_admin_api::render_stack_error;
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
        "/admin/api/v1/echo",
        post(|b: Bytes| async move { b.len().to_string() }),
    );
    let cfg = HttpConfig {
        body_limit: 4,
        ..HttpConfig::default()
    };
    apply_middleware(routes, &cfg, ErrorBodies::uniform(render_stack_error))
}

fn envelope(code: &str, message: &str) -> Value {
    json!({ "error": { "code": code, "message": message } })
}

#[tokio::test]
async fn every_stack_error_has_a_stable_code() {
    let cases = [
        (StackError::BadRequest, "bad_request", "bad request"),
        (StackError::NotFound, "not_found", "not found"),
        (
            StackError::MethodNotAllowed,
            "method_not_allowed",
            "method not allowed",
        ),
        (
            StackError::PayloadTooLarge,
            "payload_too_large",
            "payload too large",
        ),
        (StackError::Timeout, "timeout", "request timed out"),
        (
            StackError::Unavailable,
            "unavailable",
            "service unavailable",
        ),
        (StackError::Internal, "internal", "internal server error"),
    ];
    for (err, code, message) in cases {
        let body = to_bytes(render_stack_error(err).into_body(), 4096)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value, envelope(code, message));
    }
}

#[tokio::test]
async fn unknown_route_is_an_enveloped_404() {
    let req = Request::get("/admin/api/v1/missing")
        .body(Body::empty())
        .unwrap();
    let got = call(app(), req).await;
    assert_eq!(
        got,
        (StatusCode::NOT_FOUND, envelope("not_found", "not found"))
    );
}

#[tokio::test]
async fn wrong_method_is_an_enveloped_405() {
    let req = Request::get("/admin/api/v1/echo")
        .body(Body::empty())
        .unwrap();
    let got = call(app(), req).await;
    assert_eq!(
        got,
        (
            StatusCode::METHOD_NOT_ALLOWED,
            envelope("method_not_allowed", "method not allowed")
        )
    );
}

#[tokio::test]
async fn oversized_body_is_an_enveloped_413() {
    let req = Request::post("/admin/api/v1/echo")
        .body(Body::from(vec![0u8; 5]))
        .unwrap();
    let got = call(app(), req).await;
    assert_eq!(
        got,
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            envelope("payload_too_large", "payload too large")
        )
    );
}
