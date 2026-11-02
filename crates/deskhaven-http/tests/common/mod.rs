//! Helpers for tests against real bound listeners.
// Each test binary uses a different subset.
#![allow(dead_code, clippy::unwrap_used, clippy::panic)]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use axum::Router;
use axum::routing::get;
use deskhaven_http::{Apps, ClientIp, ErrorBodies, HttpConfig, ListenConfig, Listeners};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpSocket, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub const DEADLINE: Duration = Duration::from_secs(5);
pub const HANDLER_DELAY: Duration = Duration::from_millis(300);
/// Larger than any loopback socket buffer, so an unread response blocks writes.
pub const BIG_BODY: usize = 64 * 1024 * 1024;

pub fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

pub fn apps() -> Apps {
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
        )
        .route("/api/big", get(|| async { vec![0u8; BIG_BODY] }));
    let admin = Router::new()
        .route("/admin/ping", get(|| async { "admin-pong" }))
        .route("/shared", get(|| async { "admin" }));
    Apps {
        client_api,
        admin,
        errors: ErrorBodies::default(),
    }
}

pub fn request(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
}

/// Sends `request(path)` and reads until EOF. A connection the server closes
/// without answering yields an empty string.
pub async fn send_on(mut stream: TcpStream, path: &str) -> String {
    if stream.write_all(request(path).as_bytes()).await.is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).into_owned()
}

pub async fn http_get(addr: SocketAddr, path: &str) -> String {
    send_on(TcpStream::connect(addr).await.unwrap(), path).await
}

/// Connects from a chosen loopback source address (Linux routes all of 127/8).
pub async fn connect_from(source: [u8; 4], addr: SocketAddr) -> TcpStream {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::from((source, 0))).unwrap();
    socket.connect(addr).await.unwrap()
}

/// Opens a connection that sends only a partial request line.
pub async fn slow_header(addr: SocketAddr) -> TcpStream {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(b"GET /api/ip HTTP/1.1\r\n").await.unwrap();
    stream
}

pub struct Running {
    pub client_api: SocketAddr,
    pub admin: Option<SocketAddr>,
    token: CancellationToken,
    task: JoinHandle<()>,
}

impl Running {
    pub async fn start(listen: ListenConfig, cfg: HttpConfig) -> Self {
        Self::start_with(listen, cfg, apps()).await
    }

    pub async fn start_with(listen: ListenConfig, cfg: HttpConfig, apps: Apps) -> Self {
        let listeners = Listeners::bind(&listen).await.unwrap();
        let client_api = listeners.client_api_addr();
        let admin = listeners.admin_addr();
        let token = CancellationToken::new();
        let shutdown = token.clone();
        let task = tokio::spawn(async move { listeners.serve(apps, &cfg, shutdown).await });
        Self {
            client_api,
            admin,
            token,
            task,
        }
    }

    /// Cancels and returns how long `serve` took to return.
    pub async fn stop(self) -> Duration {
        let started = Instant::now();
        self.token.cancel();
        tokio::time::timeout(DEADLINE, self.task)
            .await
            .unwrap()
            .unwrap();
        started.elapsed()
    }
}

pub fn single() -> ListenConfig {
    ListenConfig {
        client_api: loopback(),
        admin: None,
        admin_public_url: None,
    }
}

pub fn split(url: Option<&str>) -> ListenConfig {
    ListenConfig {
        client_api: loopback(),
        admin: Some(loopback()),
        admin_public_url: url.map(str::to_owned),
    }
}
