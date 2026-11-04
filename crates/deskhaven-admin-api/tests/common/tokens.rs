//! Helpers of the API token tests: console users, created tokens, the real `--deploy`
//! endpoint and the audit snapshots.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::{Method, StatusCode};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::api_tokens::{NewApiToken, Scope};
use deskhaven_core::users::Role;
use deskhaven_db::testing::TestDb;
use deskhaven_db::time;
use deskhaven_http::{ErrorBodies, HttpConfig, apply_middleware};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::{Console, Harness, Reply, harness, request};

pub const TOKENS: &str = "/admin/api/v1/api-tokens";
pub const HOUR_MS: i64 = 3_600_000;

/// A harness with one admin, `adm`, logged in.
pub async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

pub fn now_ms() -> i64 {
    time::now().timestamp_millis()
}

/// Creates a token expiring in an hour; returns the response body (`token`, `secret`).
pub async fn create(h: &Harness, console: &Console, name: &str, scopes: &[&str]) -> Value {
    let body = json!({ "name": name, "scopes": scopes, "expires_at": now_ms() + HOUR_MS });
    let reply = h.write(Method::POST, TOKENS, console, body).await;
    assert_eq!(reply.status, 201, "{}", reply.body);
    reply.body
}

pub fn id_of(created: &Value) -> i64 {
    created["token"]["id"].as_i64().unwrap()
}

pub fn revoke_path(id: i64) -> String {
    format!("{TOKENS}/{id}/revoke")
}

/// `POST /api/devices/deploy` through the real client API stack with `token`.
pub async fn deploy(h: &Harness, token: &str) -> (StatusCode, Value) {
    let router = apply_middleware(
        deskhaven_client_api::router().with_state(Arc::clone(&h.state)),
        &HttpConfig::default(),
        ErrorBodies::uniform(deskhaven_client_api::render_stack_error),
    );
    let bearer = format!("Bearer {token}");
    let body = json!({ "id": "123456789", "uuid": "dXVpZA==", "pk": "AAAA" });
    let headers = [("authorization", bearer.as_str())];
    let req = request(Method::POST, "/api/devices/deploy", &headers, Some(body));
    let response = router.oneshot(req).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// A token without creator, as a future CLI would issue one.
pub async fn system_token(h: &Harness) -> i64 {
    let new = NewApiToken {
        name: "system".to_owned(),
        scopes: [Scope::Deploy].into(),
        created_by: None,
        expires_at: None,
    };
    h.state.api_tokens.create(new).await.unwrap().0.id
}

pub fn names(reply: &Reply) -> Vec<String> {
    let items = reply.body["items"].as_array().unwrap();
    let name = |t: &Value| t["name"].as_str().unwrap().to_owned();
    items.iter().map(name).collect()
}

pub fn snapshot(text: &Option<String>) -> Value {
    serde_json::from_str(text.as_deref().unwrap()).unwrap()
}
