//! Connection-level limits on real listeners: header-read timeout, write stall
//! timeout, global and per-IP connection caps, header size, drain deadline.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::{DEADLINE, Running, connect_from, http_get, request, send_on, single, slow_header};
use deskhaven_http::{HttpConfig, TrustedProxies};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const SETTLE: Duration = Duration::from_millis(150);

/// Review H-a: a client that requests a large response and never reads it is
/// disconnected after `write_stall_timeout`, which frees its slot.
#[tokio::test]
async fn slow_reader_is_disconnected_and_frees_its_slot() {
    let cfg = HttpConfig {
        max_connections: 1,
        write_stall_timeout: Duration::from_millis(300),
        ..HttpConfig::default()
    };
    let srv = Running::start(single(), cfg).await;
    let mut reader = TcpStream::connect(srv.client_api).await.unwrap();
    reader
        .write_all(request("/api/big").as_bytes())
        .await
        .unwrap();
    tokio::time::sleep(SETTLE).await;

    let res = tokio::time::timeout(DEADLINE, http_get(srv.client_api, "/api/ip"))
        .await
        .expect("slot still pinned by the slow reader");
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    drop(reader);
    srv.stop().await;
}

fn per_ip(limit: usize, trusted: &[&str]) -> HttpConfig {
    HttpConfig {
        max_connections_per_ip: limit,
        header_read_timeout: Duration::from_secs(30),
        // Holders are still open when the test stops the server.
        drain_timeout: Duration::from_millis(200),
        trusted_proxies: TrustedProxies::new(trusted.iter().map(|c| c.parse().unwrap())),
        ..HttpConfig::default()
    }
}

/// Review H-a: one address cannot take more than its share of slots.
#[tokio::test]
async fn per_ip_cap_rejects_excess_connections_from_one_address() {
    let srv = Running::start(single(), per_ip(2, &[])).await;
    let _a = slow_header(srv.client_api).await;
    let _b = slow_header(srv.client_api).await;
    tokio::time::sleep(SETTLE).await;

    let third = TcpStream::connect(srv.client_api).await.unwrap();
    let res = tokio::time::timeout(DEADLINE, send_on(third, "/api/ip"))
        .await
        .unwrap();
    assert_eq!(res, "", "third connection from one IP was served");

    let other = connect_from([127, 0, 0, 2], srv.client_api).await;
    let res = send_on(other, "/api/ip").await;
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    assert!(res.ends_with("127.0.0.2"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn per_ip_cap_slot_is_released_when_a_connection_closes() {
    let srv = Running::start(single(), per_ip(1, &[])).await;
    let holder = slow_header(srv.client_api).await;
    tokio::time::sleep(SETTLE).await;
    drop(holder);
    tokio::time::sleep(SETTLE).await;
    let res = http_get(srv.client_api, "/api/ip").await;
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    srv.stop().await;
}

#[tokio::test]
async fn trusted_proxy_is_exempt_from_per_ip_cap() {
    let srv = Running::start(single(), per_ip(1, &["127.0.0.1/32"])).await;
    let _a = slow_header(srv.client_api).await;
    let _b = slow_header(srv.client_api).await;
    tokio::time::sleep(SETTLE).await;
    let res = http_get(srv.client_api, "/api/ip").await;
    assert!(res.starts_with("HTTP/1.1 200"), "{res}");
    srv.stop().await;
}

/// Review M-a: oversized request heads are refused; normal ones still work.
#[tokio::test]
async fn oversized_header_is_rejected() {
    let srv = Running::start(single(), HttpConfig::default()).await;
    let mut stream = TcpStream::connect(srv.client_api).await.unwrap();
    let huge = "a".repeat(300 * 1024);
    let req = format!("GET /api/ip HTTP/1.1\r\nHost: t\r\nX-Big: {huge}\r\n\r\n");
    let _ = stream.write_all(req.as_bytes()).await;
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(DEADLINE, stream.read_to_end(&mut buf))
        .await
        .unwrap();
    let res = String::from_utf8_lossy(&buf);
    assert!(
        res.is_empty() || res.starts_with("HTTP/1.1 431"),
        "oversized header accepted: {}",
        &res[..res.len().min(80)]
    );
    let ok = http_get(srv.client_api, "/api/ip").await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    srv.stop().await;
}

#[tokio::test]
async fn too_many_headers_are_rejected() {
    let cfg = HttpConfig {
        max_headers: 16,
        ..HttpConfig::default()
    };
    let srv = Running::start(single(), cfg).await;
    let mut stream = TcpStream::connect(srv.client_api).await.unwrap();
    let extra: String = (0..30).map(|i| format!("X-H{i}: v\r\n")).collect();
    let req = format!("GET /api/ip HTTP/1.1\r\nHost: t\r\n{extra}\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    let res = String::from_utf8_lossy(&buf);
    assert!(res.is_empty() || res.starts_with("HTTP/1.1 431"), "{res}");
    srv.stop().await;
}

#[test]
fn header_read_timeout_defaults_to_nginx_client_header_timeout() {
    assert_eq!(
        HttpConfig::default().header_read_timeout,
        Duration::from_secs(60)
    );
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
        // The holder must outlive the assertion window below.
        header_read_timeout: Duration::from_secs(30),
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
