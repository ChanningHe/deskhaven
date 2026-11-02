//! Real bound listeners: ConnectInfo, split-admin topology, graceful shutdown.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use axum::routing::get;
use deskhaven_http::{Apps, ClientIp, HttpConfig, ListenConfig, Listeners, ServeError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const DEADLINE: Duration = Duration::from_secs(5);
const ADMIN_URL: &str = "https://admin.example.com/";

fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

fn apps() -> Apps {
    let client_api = Router::new()
        .route(
            "/api/ip",
            get(|ClientIp(ip): ClientIp| async move { ip.to_string() }),
        )
        .route(
            "/api/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                "slow-done"
            }),
        );
    let admin = Router::new().route("/admin/ping", get(|| async { "admin-pong" }));
    Apps { client_api, admin }
}

async fn http_get(addr: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = String::new();
    stream.read_to_string(&mut buf).await.unwrap();
    buf
}

struct Running {
    client_api: SocketAddr,
    admin: Option<SocketAddr>,
    token: CancellationToken,
    task: JoinHandle<Result<(), ServeError>>,
}

impl Running {
    async fn start(cfg: ListenConfig) -> Self {
        let listeners = Listeners::bind(&cfg).await.unwrap();
        let client_api = listeners.client_api_addr();
        let admin = listeners.admin_addr();
        let token = CancellationToken::new();
        let shutdown = token.clone();
        let task = tokio::spawn(async move {
            listeners
                .serve(apps(), &HttpConfig::default(), shutdown)
                .await
        });
        Self {
            client_api,
            admin,
            token,
            task,
        }
    }

    async fn stop(self) {
        self.token.cancel();
        let result = tokio::time::timeout(DEADLINE, self.task)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_ok(), "{result:?}");
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
    let srv = Running::start(single()).await;
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
async fn split_admin_redirects_client_root_and_separates_routes() {
    let srv = Running::start(split(Some(ADMIN_URL))).await;
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
    let srv = Running::start(split(None)).await;
    let res = http_get(srv.client_api, "/").await;
    assert!(res.starts_with("HTTP/1.1 404"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn graceful_shutdown_finishes_in_flight_requests() {
    let srv = Running::start(single()).await;
    let addr = srv.client_api;
    let in_flight = tokio::spawn(async move { http_get(addr, "/api/slow").await });
    tokio::time::sleep(Duration::from_millis(100)).await;

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
async fn invalid_admin_public_url_is_rejected() {
    let err = Listeners::bind(&split(Some("https://bad\nheader")))
        .await
        .err()
        .unwrap();
    assert!(matches!(err, ServeError::InvalidAdminPublicUrl), "{err:?}");
}
