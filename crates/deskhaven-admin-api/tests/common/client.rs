//! The official client API (the real router and middleware stack) as a user's client would
//! call it, on the same state as the console harness.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::{Method, StatusCode};
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::users::Password;
use deskhaven_http::{ErrorBodies, HttpConfig, apply_middleware};
use serde_json::Value;
use tower::ServiceExt;

use super::{Harness, PASSWORD, request};

/// A client session of `username` (an existing local user); returns its bearer token.
pub async fn client_token(h: &Harness, username: &str) -> String {
    let auth = h
        .state
        .users
        .authenticate_local(username, &Password::new(PASSWORD))
        .await
        .unwrap();
    let device = ClientDevice {
        rustdesk_id: "123456789".to_owned(),
        uuid: "uuid-1".to_owned(),
        info: None,
    };
    let (_, token) = h
        .state
        .sessions
        .create_client(auth.user.id, &auth.stamp, &device, None)
        .await
        .unwrap();
    token.expose().to_owned()
}

/// `method path` (with its query) against the client API as the holder of `token`; the
/// status and the JSON body (`Null` when empty).
pub async fn client_call(
    h: &Harness,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let router = apply_middleware(
        deskhaven_client_api::router().with_state(Arc::clone(&h.state)),
        &HttpConfig::default(),
        ErrorBodies::uniform(deskhaven_client_api::render_stack_error),
    );
    let bearer = format!("Bearer {token}");
    let response = router
        .oneshot(request(
            method,
            path,
            &[("authorization", bearer.as_str())],
            body,
        ))
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let json = match bytes.is_empty() {
        true => Value::Null,
        false => serde_json::from_slice(&bytes).unwrap(),
    };
    (status, json)
}
