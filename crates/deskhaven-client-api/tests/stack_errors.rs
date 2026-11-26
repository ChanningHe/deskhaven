//! Errors produced by the HTTP stack itself use the client contract shape.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use axum::body::to_bytes;
use axum::http::header;
use deskhaven_client_api::render_stack_error;
use deskhaven_http::StackError;
use serde_json::{Value, json};

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
        assert_eq!(res.headers()[header::CONTENT_TYPE], "application/json");
        let body = to_bytes(res.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value, json!({ "error": message }));
    }
}
