//! The admin API is mounted in the real server: `/admin/api/v1/*` reaches its gate on
//! the single listener (not the console's SPA fallback), with the admin error envelope.
#![allow(clippy::unwrap_used)]

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::fixture::{Listen, Request};
use serde_json::{Value, json};

async fn single() -> TestApp {
    TestApp::start_with_options(TestOptions {
        listen: Listen::Single,
        ..TestOptions::default()
    })
    .await
    .unwrap()
}

fn body(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

#[tokio::test]
async fn me_without_a_session_is_an_enveloped_401() {
    // Arrange
    let app = single().await;

    // Act
    let reply = app
        .send(&Request::new("GET", "/admin/api/v1/me"))
        .await
        .unwrap();

    // Assert
    assert_eq!(reply.status, 401);
    assert_eq!(reply.headers["cache-control"], "no-store");
    assert_eq!(
        body(&reply.body),
        json!({ "error": { "code": "unauthorized", "message": "authentication required" } })
    );
}

#[tokio::test]
async fn unknown_admin_api_paths_are_enveloped_404s_not_the_console() {
    // Arrange
    let app = single().await;

    // Act
    let reply = app
        .send(&Request::new("GET", "/admin/api/v1/nope"))
        .await
        .unwrap();

    // Assert
    assert_eq!(reply.status, 404);
    assert_eq!(
        body(&reply.body),
        json!({ "error": { "code": "not_found", "message": "not found" } })
    );
}
