//! Real bound listeners: ConnectInfo, split-admin topology, connection limits
//! and graceful shutdown.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use deskhaven_http::{
    Apps, ClientIp, ErrorBodies, HttpConfig, ListenConfig, Listeners, ServeError,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const DEADLINE: Duration = Duration::from_secs(5);
const ADMIN_URL: &str = "https://admin.example.com/";
const HANDLER_DELAY: Duration = Duration::from_millis(300);

fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

fn apps() -> Apps {
    let client_api = Router::new()
        .route(
            "/api/ip",
            get(|ip: ClientIp| async move { ip.addr.to_string() }),
        )
        .route("/shared", get(|| async { "client-wins" }))
        .route(
            "/api/slow",
            get(|| async {
                tokio::time::sleep(HANDLER_DELAY).await;
                "slow-done"
            }),
        )
        .route(
            "/api/stuck",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                "never"
            }),
        );
    let admin = Router::new()
        .route("/admin/ping", get(|| async { "admin-pong" }))
        .route("/shared", get(|| async { "admin" }));
    Apps {
        client_api,
        admin,
        errors: ErrorBodies::default(),
    }
}

fn request(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
}

async fn http_get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(request(path).as_bytes()).await.unwrap();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).await.unwrap();
    buf
}

/// Opens a connection that sends only a partial request line.
async fn slow_header(addr: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(b"GET /api/ip HTTP/1.1\r\n").await.unwrap();
    stream
}

struct Running {
    client_api: SocketAddr,
    admin: Option<SocketAddr>,
    token: CancellationToken,
    task: JoinHandle<()>,
}

impl Running {
    async fn start(listen: ListenConfig, cfg: HttpConfig) -> Self {
        let listeners = Listeners::bind(&listen).await.unwrap();
        let client_api = listeners.client_api_addr();
        let admin = listeners.admin_addr();
        let token = CancellationToken::new();
        let shutdown = token.clone();
        let task = tokio::spawn(async move { listeners.serve(apps(), &cfg, shutdown).await });
        Self {
            client_api,
            admin,
            token,
            task,
        }
    }

    /// Cancels and returns how long `serve` took to return.
    async fn stop(self) -> Duration {
        let started = Instant::now();
        self.token.cancel();
        tokio::time::timeout(DEADLINE, self.task)
            .await
            .unwrap()
            .unwrap();
        started.elapsed()
    }
}

fn single() -> ListenConfig {
    ListenConfig {
        client_api: loopback(),
        admin: None,
        admin_public_url: None,
    }
}

fn split(url: Option<&str>) -> ListenConfig {
    ListenConfig {
        client_api: loopback(),
        admin: Some(loopback()),
        admin_public_url: url.map(str::to_owned),
    }
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

/// Review H2: a client that never finishes its request head is disconnected.
#[tokio::test]
async fn slow_header_connection_is_closed_by_header_read_timeout() {
    let cfg = HttpConfig {
        header_read_timeout: Duration::from_millis(200),
        ..HttpConfig::default()
    };
    let srv = Running::start(single(), cfg).await;
    let mut stream = slow_header(srv.client_api).await;
    let mut buf = Vec::new();
    let read = tokio::time::timeout(DEADLINE, stream.read_to_end(&mut buf)).await;
    assert!(read.is_ok(), "connection still open after header timeout");
    srv.stop().await;
}

/// Review H2: idle keep-alive connections are bounded by the same timer.
#[tokio::test]
async fn idle_keep_alive_connection_is_closed() {
    let cfg = HttpConfig {
        header_read_timeout: Duration::from_millis(200),
        ..HttpConfig::default()
    };
    let srv = Running::start(single(), cfg).await;
    let mut stream = TcpStream::connect(srv.client_api).await.unwrap();
    let keep_alive = "GET /api/ip HTTP/1.1\r\nHost: test\r\n\r\n";
    stream.write_all(keep_alive.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let read = tokio::time::timeout(DEADLINE, stream.read_to_end(&mut buf)).await;
    assert!(read.is_ok(), "idle keep-alive connection never closed");
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 200"));
    srv.stop().await;
}

/// Review H2: stuck connections cannot hold `serve` past the drain deadline.
#[tokio::test]
async fn shutdown_with_stuck_connections_returns_within_drain_deadline() {
    let cfg = HttpConfig {
        request_timeout: Duration::from_secs(3600),
        drain_timeout: Duration::from_millis(300),
        ..HttpConfig::default()
    };
    let srv = Running::start(single(), cfg).await;
    let _slow = slow_header(srv.client_api).await;
    let mut stuck = TcpStream::connect(srv.client_api).await.unwrap();
    stuck
        .write_all(request("/api/stuck").as_bytes())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let took = srv.stop().await;
    assert!(took < Duration::from_secs(2), "serve took {took:?}");
    let mut buf = Vec::new();
    let read = tokio::time::timeout(DEADLINE, stuck.read_to_end(&mut buf)).await;
    assert!(read.is_ok(), "stuck connection was not dropped");
}

#[tokio::test]
async fn connection_cap_defers_extra_connections() {
    let cfg = HttpConfig {
        max_connections: 1,
        ..HttpConfig::default()
    };
    let srv = Running::start(single(), cfg).await;
    let holder = slow_header(srv.client_api).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let addr = srv.client_api;
    let mut waiting = tokio::spawn(async move { http_get(addr, "/api/ip").await });
    let early = tokio::time::timeout(Duration::from_millis(300), &mut waiting).await;
    assert!(early.is_err(), "second connection served above the cap");

    drop(holder);
    let res = tokio::time::timeout(DEADLINE, waiting)
        .await
        .unwrap()
        .unwrap();
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    srv.stop().await;
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
