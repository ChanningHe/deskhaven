//! M4-05: admission limits of the main TCP port and the websocket port.
//!
//! Every accepted main-port connection costs an X25519 key generation and two Ed25519
//! signatures (the KeyExchange offer, M1-07) before the client has said anything. Upstream
//! accepted without any limit. Now a main-port connection is refused (closed at once, no
//! offer) when its source has used up its accept budget (token bucket) or already has too many
//! connections that have not sent their first frame. When the server as a whole has too many
//! of those, the oldest one is closed to make room: a silent flood cannot keep a client that
//! answers within a round trip out. Established connections are capped per source and in all,
//! on the main and the websocket port.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{AcceptLimits, SignalConfig};
use hbb_common::{
    rendezvous_proto::punch_hole_response::Failure::ID_NOT_EXIST,
    tokio::{self, io::AsyncReadExt, net::TcpStream, time::Duration},
};
use std::net::IpAddr;
use support::{
    Conn, Recv, Rendezvous, TcpClient, TestServer, WsClient, expect_msg, next_client_ip, read_offer,
};

/// Long enough for a refused connection to be closed, short enough to keep the tests fast.
const WAIT: Duration = Duration::from_secs(5);

async fn server_with(limits: AcceptLimits) -> TestServer {
    TestServer::start_with(|cfg| SignalConfig {
        accept_limits: limits,
        ..cfg
    })
    .await
}

/// A connection from `ip` that got the KeyExchange offer.
async fn admitted(server: &TestServer, ip: IpAddr) -> TcpClient {
    let mut conn = TcpClient::connect_from(ip, server.main_addr()).await;
    read_offer(&mut conn).await;
    conn
}

/// A connection from `ip` whose first frame was served.
async fn established(server: &TestServer, ip: IpAddr) -> TcpClient {
    let mut conn = admitted(server, ip).await;
    first_frame(&mut conn, server).await;
    conn
}

/// Asserts the server closes `conn` without sending it anything (more).
async fn assert_closed(conn: &mut impl Conn) {
    let got = conn.recv_within(WAIT).await;
    assert!(matches!(got, Recv::Closed), "expected a close, got {got:?}");
}

/// Asserts the server closes a fresh connection from `ip` without sending anything.
async fn assert_refused(server: &TestServer, ip: IpAddr) {
    assert_closed(&mut TcpClient::connect_from(ip, server.main_addr()).await).await;
}

/// The first frame of `conn` (a plain punch hole request) is served.
async fn first_frame(conn: &mut impl Rendezvous, server: &TestServer) {
    conn.punch_hole_request(server.punch_req("no-such-peer"))
        .await;
    let res = expect_msg!(conn.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
}

#[tokio::test]
async fn an_ip_with_too_many_pending_handshakes_is_refused_until_one_sends_a_frame() {
    // Arrange
    let server = server_with(AcceptLimits {
        per_ip_pending: 2,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    let mut first = admitted(&server, ip).await;
    let _second = admitted(&server, ip).await;

    // Act + Assert
    assert_refused(&server, ip).await;
    admitted(&server, next_client_ip()).await;
    first_frame(&mut first, &server).await;
    admitted(&server, ip).await;
    server.stop().await;
}

/// Review H-A: silent connections from many sources fill the global pending cap; a fresh
/// source still completes its handshake, and the oldest silent connection makes room.
#[tokio::test]
async fn silent_connections_at_the_global_cap_cannot_keep_a_fresh_source_out() {
    // Arrange
    let server = server_with(AcceptLimits {
        pending: 4,
        ..AcceptLimits::default()
    })
    .await;
    let mut silent = Vec::new();
    for _ in 0..4 {
        silent.push(admitted(&server, next_client_ip()).await);
    }

    // Act
    let fresh = established(&server, next_client_ip()).await;

    // Assert
    assert_closed(&mut silent[0]).await;
    for conn in &mut silent[1..] {
        assert!(matches!(
            conn.recv_within(Duration::from_millis(200)).await,
            Recv::Quiet
        ));
    }
    drop(fresh);
    server.stop().await;
}

#[tokio::test]
async fn an_ip_that_used_up_its_accept_burst_is_refused() {
    // Arrange
    let server = server_with(AcceptLimits {
        per_ip_burst: 3,
        per_ip_per_second: 1,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    for _ in 0..3 {
        established(&server, ip).await;
    }

    // Act + Assert
    assert_refused(&server, ip).await;
    admitted(&server, next_client_ip()).await;
    server.stop().await;
}

/// Review M-A: established connections are capped per source, not only pending ones.
#[tokio::test]
async fn an_ip_with_too_many_established_connections_is_refused() {
    // Arrange
    let server = server_with(AcceptLimits {
        per_ip_connections: 2,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    let _held = [
        established(&server, ip).await,
        established(&server, ip).await,
    ];

    // Act + Assert
    assert_refused(&server, ip).await;
    established(&server, next_client_ip()).await;
    server.stop().await;
}

/// Review M-A: the server's connections in all are capped; a closed one gives its slot back.
#[tokio::test]
async fn the_server_refuses_connections_beyond_its_global_cap_until_one_closes() {
    // Arrange
    let server = server_with(AcceptLimits {
        connections: 2,
        ..AcceptLimits::default()
    })
    .await;
    let first = established(&server, next_client_ip()).await;
    let _second = established(&server, next_client_ip()).await;
    assert_refused(&server, next_client_ip()).await;

    // Act
    drop(first);

    // Assert: the server notices the close and frees the slot.
    let mut admitted_again = false;
    for _ in 0..50 {
        let mut conn = TcpClient::connect_from(next_client_ip(), server.main_addr()).await;
        if !matches!(
            conn.recv_within(Duration::from_millis(100)).await,
            Recv::Closed
        ) {
            admitted_again = true;
            break;
        }
    }
    assert!(admitted_again);
    server.stop().await;
}

/// Review M-A: the websocket port counts towards the same caps: in all at accept, per source
/// (the client IP the proxy rule resolves) after the handshake.
#[tokio::test]
async fn websocket_connections_are_capped_in_all_and_per_source() {
    // Arrange
    let server = server_with(AcceptLimits {
        per_ip_connections: 1,
        connections: 2,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    let mut first = WsClient::connect_from(ip, server.ws_addr(), &[]).await;
    first_frame(&mut first, &server).await;

    // Act + Assert: per source, after the handshake.
    assert_closed(&mut WsClient::connect_from(ip, server.ws_addr(), &[]).await).await;
    let mut other = WsClient::connect_from(next_client_ip(), server.ws_addr(), &[]).await;
    first_frame(&mut other, &server).await;
    // In all, before the handshake: the server closes the TCP connection at once.
    let mut raw = TcpStream::connect(server.ws_addr()).await.unwrap();
    let mut buf = [0u8; 16];
    let read = tokio::time::timeout(WAIT, raw.read(&mut buf))
        .await
        .unwrap();
    assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");
    server.stop().await;
}

#[tokio::test]
async fn the_default_limits_admit_a_burst_of_connections_from_one_ip() {
    // Arrange: a NAT gateway in front of many devices reconnecting after a server restart.
    let server = TestServer::start().await;
    let ip = next_client_ip();

    // Act + Assert
    let mut held = Vec::new();
    for _ in 0..32 {
        held.push(admitted(&server, ip).await);
    }
    server.stop().await;
}
