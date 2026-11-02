//! The real composition on a real listener: client API + admin API + the web
//! console as the admin side's catch-all, single-listener mode, with the CSP
//! configured through `deskhaven-http`.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use common::{Fixture, INDEX_HTML};
use deskhaven_http::{Apps, ErrorBodies, HttpConfig, ListenConfig, Listeners, StackError};
use deskhaven_web::{CSP, router_for};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

fn client_json(e: StackError) -> Response<Body> {
    let body = format!(r#"{{"error":"client:{}"}}"#, e.message());
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn admin_json(e: StackError) -> Response<Body> {
    let body = format!(r#"{{"error":{{"code":"admin:{}"}}}}"#, e.message());
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

fn apps() -> Apps {
    let client_api = Router::new().route("/api/ping", get(|| async { "client-pong" }));
    let admin = Router::new()
        .route("/admin/api/v1/ping", get(|| async { "admin-pong" }))
        .merge(router_for::<Fixture>());
    Apps {
        client_api,
        admin,
        errors: ErrorBodies {
            client_api: client_json,
            admin: admin_json,
        },
    }
}

fn config() -> HttpConfig {
    HttpConfig {
        content_security_policy: Some(HeaderValue::from_static(CSP)),
        ..HttpConfig::default()
    }
}

struct Server {
    addr: SocketAddr,
    admin: Option<SocketAddr>,
    token: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

async fn start() -> Server {
    start_with(None).await
}

/// Split mode: the console and the admin API get their own listener.
async fn start_split() -> Server {
    start_with(Some(loopback())).await
}

async fn start_with(admin: Option<SocketAddr>) -> Server {
    let listen = ListenConfig {
        client_api: loopback(),
        admin,
        admin_public_url: None,
    };
    let listeners = Listeners::bind(&listen).await.unwrap();
    let addr = listeners.client_api_addr();
    let admin = listeners.admin_addr();
    let token = CancellationToken::new();
    let shutdown = token.clone();
    let cfg = config();
    let task = tokio::spawn(async move {
        listeners.serve(apps(), &cfg, shutdown).await.unwrap();
    });
    Server {
        addr,
        admin,
        token,
        task,
    }
}

/// One request on a fresh connection; reads until the server closes it.
async fn send(addr: SocketAddr, method: &str, path: &str, accept: &str) -> String {
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: t\r\nAccept: {accept}\r\nConnection: close\r\n\r\n"
    );
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

fn content_length(response: &str) -> Option<usize> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())?
    })
}

impl Server {
    async fn get(&self, path: &str, accept: &str) -> String {
        send(self.addr, "GET", path, accept).await
    }

    async fn stop(self) {
        self.token.cancel();
        self.task.await.unwrap();
    }
}

const NAV: &str = "text/html,*/*;q=0.8";

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").map_or("", |(_, b)| b)
}

#[tokio::test]
async fn unknown_client_api_path_is_the_client_json_404_not_the_spa() {
    let srv = start().await;
    let res = srv.get("/api/unknown", NAV).await;
    srv.stop().await;

    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    assert_eq!(body(&res), r#"{"error":"client:not found"}"#);
}

#[tokio::test]
async fn unknown_admin_api_path_is_the_admin_json_404_not_the_spa() {
    let srv = start().await;
    let res = srv.get("/admin/api/unknown", NAV).await;
    srv.stop().await;

    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    assert_eq!(body(&res), r#"{"error":{"code":"admin:not found"}}"#);
}

#[tokio::test]
async fn real_api_routes_still_win_over_the_console() {
    let srv = start().await;
    let client = srv.get("/api/ping", NAV).await;
    let admin = srv.get("/admin/api/v1/ping", NAV).await;
    srv.stop().await;

    assert_eq!(body(&client), "client-pong");
    assert_eq!(body(&admin), "admin-pong");
}

#[tokio::test]
async fn console_routes_are_served_with_the_csp_and_security_headers() {
    let srv = start().await;
    let res = srv.get("/devices", NAV).await;
    let asset = srv.get("/assets/index-AbC123.js", "*/*").await;
    srv.stop().await;

    let lower = res.to_ascii_lowercase();
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    assert_eq!(body(&res), INDEX_HTML);
    assert!(lower.contains(&format!("content-security-policy: {CSP}").to_ascii_lowercase()));
    assert!(lower.contains("x-content-type-options: nosniff"), "{res}");
    assert!(lower.contains("x-frame-options: deny"), "{res}");
    assert!(lower.contains("cache-control: no-cache"), "{res}");
    // The same policy covers assets.
    assert!(
        asset
            .to_ascii_lowercase()
            .contains("content-security-policy:")
    );
}

#[tokio::test]
async fn a_missing_asset_stays_a_plain_404_outside_the_api() {
    let srv = start().await;
    let res = srv.get("/assets/x.js", NAV).await;
    srv.stop().await;

    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    assert!(!res.contains("<html"), "{res}");
}

#[tokio::test]
async fn doubled_slash_api_paths_never_get_the_spa() {
    let srv = start().await;
    for path in ["//api/x", "/admin//api/x", "/%2fapi/x"] {
        let res = srv.get(path, NAV).await;
        assert!(res.starts_with("HTTP/1.1 404"), "{path}: {res}");
        assert!(!res.contains("<html"), "{path}: {res}");
    }
    srv.stop().await;
}

/// Split mode: the admin listener does not serve the client API, and the
/// console must not answer for it either.
#[tokio::test]
async fn split_admin_listener_gets_a_404_for_client_api_paths_not_the_spa() {
    let srv = start_split().await;
    let admin = srv.admin.unwrap();
    let res = send(admin, "GET", "/api/unknown", NAV).await;
    let spa = send(admin, "GET", "/devices", NAV).await;
    srv.stop().await;

    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    assert!(!res.contains("<html"), "{res}");
    assert!(spa.starts_with("HTTP/1.1 200"), "{spa}");
    assert_eq!(body(&spa), INDEX_HTML);
}

#[tokio::test]
async fn head_reports_the_same_content_length_as_get_with_no_body() {
    let srv = start().await;
    for path in ["/devices", "/assets/index-AbC123.js"] {
        let get = send(srv.addr, "GET", path, NAV).await;
        let head = send(srv.addr, "HEAD", path, NAV).await;

        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert_eq!(body(&head), "", "{path}");
        assert_eq!(content_length(&head), Some(body(&get).len()), "{path}");
        assert_eq!(content_length(&get), Some(body(&get).len()), "{path}");
    }
    srv.stop().await;
}
