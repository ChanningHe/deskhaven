//! `GET /api/version` (AGPL-3.0 section 13 source offer): the values the
//! fixtures `version/*` cannot pin because they change with the build.
#![allow(clippy::unwrap_used)]

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::fixture::{Listen, Request};
use serde_json::{Value, json};

/// Every workspace crate shares the version, this crate included.
const WORKSPACE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::test]
async fn reports_the_workspace_version_and_the_source_url() {
    // Arrange
    let app = TestApp::start().await.unwrap();

    // Act
    let reply = app
        .send(&Request::new("GET", "/api/version"))
        .await
        .unwrap();

    // Assert
    assert_eq!(reply.status, 200);
    let body: Value = reply.json().unwrap();
    assert_eq!(
        body,
        json!({
            "version": WORKSPACE_VERSION,
            "source_url": "https://git.homielab.cc/ChanningHe/deskhaven"
        })
    );
}

#[tokio::test]
async fn is_served_to_anonymous_head_and_on_the_single_listener() {
    // Arrange
    let app = TestApp::start_with_options(TestOptions {
        listen: Listen::Single,
        ..TestOptions::default()
    })
    .await
    .unwrap();

    // Act
    let get = app
        .send(&Request::new("GET", "/api/version"))
        .await
        .unwrap();
    let head = app
        .send(&Request::new("HEAD", "/api/version"))
        .await
        .unwrap();

    // Assert: no credentials needed, and the console fallback does not shadow it.
    assert_eq!(get.status, 200);
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());
}
