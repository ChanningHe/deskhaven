//! Real bound listeners: ConnectInfo, split-admin topology, route composition
//! and graceful shutdown.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::body::Body;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use common::{DEADLINE, HANDLER_DELAY, Running, apps, http_get, loopback, single, split};
use deskhaven_http::{ErrorBodies, HttpConfig, ListenConfig, Listeners, ServeError, StackError};
use tokio::net::TcpStream;

const ADMIN_URL: &str = "https://admin.example.com/";

fn json_error(e: StackError) -> Response<Body> {
    let body = format!(r#"{{"error":"{}"}}"#, e.message());
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// Review L-d: even with a catch-all admin fallback (the future SPA), an
/// unmatched `/api/*` path is the client API's JSON 404, never the SPA.
#[tokio::test]
async fn unmatched_client_api_path_is_a_client_404_despite_admin_catch_all() {
    let mut apps = apps();
    apps.admin = apps
        .admin
        .fallback(get(|| async { (StatusCode::OK, "spa") }));
    apps.errors = ErrorBodies::uniform(json_error);
    let srv = Running::start_with(single(), HttpConfig::default(), apps).await;

    let res = http_get(srv.client_api, "/api/unknown").await;
    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    assert!(res.ends_with(r#"{"error":"not found"}"#), "{res}");
    let res = http_get(srv.client_api, "/console/users").await;
    assert!(res.ends_with("spa"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn single_listener_serves_client_and_admin_with_connect_info() {
    let srv = Running::start(single(), HttpConfig::default()).await;
    assert!(srv.admin.is_none());

    let res = http_get(srv.client_api, "/api/ip").await;
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    assert!(res.ends_with("127.0.0.1"), "{res}");
    assert!(res.to_ascii_lowercase().contains("x-request-id:"), "{res}");

    let res = http_get(srv.client_api, "/admin/ping").await;
    assert!(res.ends_with("admin-pong"), "{res}");

    let res = http_get(srv.client_api, "/").await;
    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn single_listener_overlapping_route_is_served_by_client_api() {
    let srv = Running::start(single(), HttpConfig::default()).await;
    let res = http_get(srv.client_api, "/shared").await;
    assert!(res.ends_with("client-wins"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn split_admin_redirects_client_root_and_separates_routes() {
    let srv = Running::start(split(Some(ADMIN_URL)), HttpConfig::default()).await;
    let admin = srv.admin.unwrap();

    let res = http_get(srv.client_api, "/").await;
    assert!(res.starts_with("HTTP/1.1 302"), "{res}");
    assert!(res.contains(&format!("location: {ADMIN_URL}")), "{res}");

    let res = http_get(srv.client_api, "/admin/ping").await;
    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    let res = http_get(admin, "/admin/ping").await;
    assert!(res.ends_with("admin-pong"), "{res}");
    let res = http_get(admin, "/api/ip").await;
    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn split_admin_without_public_url_gives_404_on_root() {
    let srv = Running::start(split(None), HttpConfig::default()).await;
    let res = http_get(srv.client_api, "/").await;
    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn graceful_shutdown_finishes_in_flight_requests() {
    let srv = Running::start(single(), HttpConfig::default()).await;
    let addr = srv.client_api;
    let in_flight = tokio::spawn(async move { http_get(addr, "/api/slow").await });
    tokio::time::sleep(HANDLER_DELAY / 3).await;

    srv.stop().await;
    let res = tokio::time::timeout(DEADLINE, in_flight)
        .await
        .unwrap()
        .unwrap();
    assert!(res.ends_with("slow-done"), "{res}");
    assert!(TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn bind_conflict_is_reported_with_listener_name() {
    let taken = tokio::net::TcpListener::bind(loopback()).await.unwrap();
    let cfg = ListenConfig {
        client_api: taken.local_addr().unwrap(),
        ..single()
    };
    let err = Listeners::bind(&cfg).await.err().unwrap();
    assert!(
        matches!(
            err,
            ServeError::Bind {
                name: "client_api",
                ..
            }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn admin_public_url_must_be_absolute_http_url() {
    for bad in [
        "https://bad\nheader",
        "/relative",
        "admin.example.com",
        "javascript:alert(1)",
        "ftp://admin.example.com/",
    ] {
        let err = Listeners::bind(&split(Some(bad))).await.err();
        assert!(
            matches!(err, Some(ServeError::InvalidAdminPublicUrl)),
            "{bad}: {err:?}"
        );
    }
}

#[tokio::test]
async fn admin_public_url_is_ignored_when_not_split() {
    let cfg = ListenConfig {
        admin_public_url: Some("not a url".to_owned()),
        ..single()
    };
    let srv = Running::start(cfg, HttpConfig::default()).await;
    let res = http_get(srv.client_api, "/").await;
    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    srv.stop().await;
}

/// Review round 3 LOW: an invalid limit stops `serve` before anything is served.
#[tokio::test]
async fn invalid_config_is_rejected_before_serving() {
    let listeners = Listeners::bind(&single()).await.unwrap();
    let cfg = HttpConfig {
        max_header_bytes: 1024,
        ..HttpConfig::default()
    };
    let token = tokio_util::sync::CancellationToken::new();
    let result = tokio::time::timeout(DEADLINE, listeners.serve(apps(), &cfg, token))
        .await
        .unwrap();
    match result {
        Err(ServeError::InvalidConfig(err)) => assert_eq!(err.field, "max_header_bytes"),
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}
