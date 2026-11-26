//! Errors produced by the HTTP stack itself use the admin envelope.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use axum::body::to_bytes;
use axum::http::header;
use deskhaven_admin_api::render_stack_error;
use deskhaven_http::StackError;
use serde_json::{Value, json};

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
        let res = render_stack_error(err);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value, envelope(code, message));
    }
}
