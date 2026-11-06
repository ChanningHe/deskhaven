//! M4-05: admission limits of the main TCP port and the websocket port.
//!
//! Every accepted main-port connection costs an X25519 key generation and two Ed25519
//! signatures (the KeyExchange offer, M1-07) before the client has said anything. Upstream
//! accepted without any limit. Per-source limits refuse (they only hurt that source): the
//! accept budget (token bucket), connections that have not sent their first frame yet, and
//! connections in all. Global caps never refuse a new connection: when one is full, the least
//! valuable existing connection is closed instead (a pending handshake before an idle
//! connection without a registered peer, before a registered peer's connection).

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{AcceptLimits, SignalConfig};
use hbb_common::{
    rendezvous_proto::{
        punch_hole_response::Failure::ID_NOT_EXIST, register_pk_response::Result::OK,
    },
    tokio::{
        self,
        net::{TcpSocket, TcpStream},
        time::Duration,
    },
};
use std::net::{IpAddr, SocketAddr};
use support::{
    Conn, PeerKeys, Recv, Rendezvous, TcpClient, TestServer, WsClient, expect_msg, next_client_ip,
    read_offer,
};

/// Long enough for a refused connection to be closed, short enough to keep the tests fast.
const WAIT: Duration = Duration::from_secs(5);
/// How long a connection that must stay open is watched.
const QUIET: Duration = Duration::from_millis(300);

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

/// A bare TCP connection from `ip` to the websocket port that never starts its handshake.
async fn silent_ws(server: &TestServer, ip: IpAddr) -> TcpStream {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::new(ip, 0)).unwrap();
    socket.connect(server.ws_addr()).await.unwrap()
}

/// Asserts the server closes `conn` without sending it anything (more).
async fn assert_closed(conn: &mut impl Conn) {
    let got = conn.recv_within(WAIT).await;
    assert!(matches!(got, Recv::Closed), "expected a close, got {got:?}");
}

/// Asserts `conn` is still open (nothing arrives, no close).
async fn assert_open(conn: &mut impl Conn) {
    let got = conn.recv_within(QUIET).await;
    assert!(
        matches!(got, Recv::Quiet),
        "expected an open connection, got {got:?}"
    );
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
    let _fresh = established(&server, next_client_ip()).await;

    // Assert
    assert_closed(&mut silent[0]).await;
    for conn in &mut silent[1..] {
        assert_open(conn).await;
    }
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

/// Re-review HIGH-2: kept-alive connections from many sources fill the global connection cap;
/// a fresh client still gets in and the connection idle the longest makes room.
#[tokio::test]
async fn established_keep_alive_connections_at_the_global_cap_cannot_refuse_a_fresh_client() {
    // Arrange
    let server = server_with(AcceptLimits {
        connections: 4,
        ..AcceptLimits::default()
    })
    .await;
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(established(&server, next_client_ip()).await);
    }

    // Act
    let _fresh = established(&server, next_client_ip()).await;

    // Assert
    assert_closed(&mut held[0]).await;
    for conn in &mut held[1..] {
        assert_open(conn).await;
    }
    server.stop().await;
}

/// Re-review HIGH-2: a registered peer's connection is the last to go.
#[tokio::test]
async fn registered_online_peers_survive_eviction_while_idle_unregistered_ones_go() {
    // Arrange: the registered peer is the oldest connection.
    let server = server_with(AcceptLimits {
        connections: 3,
        ..AcceptLimits::default()
    })
    .await;
    let mut peer = admitted(&server, next_client_ip()).await;
    assert_eq!(
        peer.register_pk("evict-peer-1", &PeerKeys::random()).await,
        OK
    );
    let mut idle = established(&server, next_client_ip()).await;
    let mut recent = established(&server, next_client_ip()).await;

    // Act
    let _fresh = established(&server, next_client_ip()).await;

    // Assert
    assert_closed(&mut idle).await;
    assert_open(&mut peer).await;
    assert_open(&mut recent).await;
    server.stop().await;
}

/// Re-review HIGH-1: silent connections to the websocket port from one source take the
/// websocket port's own pending budget, never the main port's clients.
#[tokio::test]
async fn silent_websocket_connections_from_one_source_cannot_refuse_a_fresh_main_port_client() {
    // Arrange
    let server = server_with(AcceptLimits {
        ws_pending: 3,
        connections: 6,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    let mut silent = Vec::new();
    for _ in 0..10 {
        silent.push(silent_ws(&server, ip).await);
    }

    // Act + Assert
    established(&server, next_client_ip()).await;
    established(&server, next_client_ip()).await;
    server.stop().await;
}

/// Review M-A: websocket connections count per source (the client IP the proxy rule
/// resolves) after the handshake.
#[tokio::test]
async fn websocket_connections_are_capped_per_source_after_the_handshake() {
    // Arrange
    let server = server_with(AcceptLimits {
        per_ip_connections: 1,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    let mut first = WsClient::connect_from(ip, server.ws_addr(), &[]).await;
    first_frame(&mut first, &server).await;

    // Act + Assert
    assert_closed(&mut WsClient::connect_from(ip, server.ws_addr(), &[]).await).await;
    let mut other = WsClient::connect_from(next_client_ip(), server.ws_addr(), &[]).await;
    first_frame(&mut other, &server).await;
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
