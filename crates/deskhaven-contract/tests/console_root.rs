//! What the client's "web console" button finds at the API server base URL
//! (`GET /`) when the console is not split off. The split redirect is the
//! fixture `root/split-redirects-to-admin`.
#![allow(clippy::unwrap_used)]

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::fixture::{Listen, Request};

const BROWSER_ACCEPT: &str = "text/html,application/xhtml+xml";
const SPLIT_ADMIN_URL: &str = "https://console.example.test/";

fn browser_get(path: &str) -> Request {
    let mut request = Request::new("GET", path);
    request
        .headers
        .insert("accept".to_owned(), BROWSER_ACCEPT.to_owned());
    request
}

async fn app_on(listen: Listen) -> TestApp {
    TestApp::start_with_options(TestOptions {
        listen,
        ..TestOptions::default()
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn single_listener_root_serves_the_console_html() {
    // Arrange
    let app = app_on(Listen::Single).await;

    // Act
    let reply = app.send(&browser_get("/")).await.unwrap();

    // Assert: index.html when web/dist is built, the 503 placeholder otherwise;
    // either way a page for the browser, never the client API's JSON 404.
    assert!(matches!(reply.status, 200 | 503), "{}", reply.status);
    let content_type = reply.headers.get("content-type").unwrap();
    assert!(content_type.to_str().unwrap().starts_with("text/html"));
    assert!(reply.headers.contains_key("content-security-policy"));
    let body = String::from_utf8_lossy(&reply.body).to_ascii_lowercase();
    assert!(body.contains("<!doctype html>"), "{body}");
}

#[tokio::test]
async fn single_listener_root_does_not_swallow_the_client_api() {
    // Arrange
    let app = app_on(Listen::Single).await;

    // Act
    let options = app
        .send(&Request::new("GET", "/api/login-options"))
        .await
        .unwrap();
    let unknown = app.send(&browser_get("/api/nope")).await.unwrap();

    // Assert
    assert_eq!(options.status, 200);
    assert_eq!(unknown.status, 404);
    let content_type = unknown.headers.get("content-type").unwrap();
    assert!(
        content_type
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
}

#[tokio::test]
async fn split_root_redirect_only_applies_to_the_root_path() {
    // Arrange
    let app = app_on(Listen::Split {
        admin_public_url: SPLIT_ADMIN_URL.to_owned(),
    })
    .await;

    // Act
    let root = app.send(&browser_get("/")).await.unwrap();
    let api = app
        .send(&Request::new("GET", "/api/login-options"))
        .await
        .unwrap();
    let other = app.send(&browser_get("/devices")).await.unwrap();

    // Assert: the console is elsewhere, so other paths are plain 404s here.
    assert_eq!(root.status, 302);
    assert_eq!(api.status, 200);
    assert_eq!(other.status, 404);
}
