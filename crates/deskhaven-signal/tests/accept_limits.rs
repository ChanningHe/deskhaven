//! M4-05: admission limits of the main TCP port.
//!
//! Every accepted main-port connection costs an X25519 key generation and two Ed25519
//! signatures (the KeyExchange offer, M1-07) before the client has said anything. Upstream
//! accepted without any limit. Now a connection is refused (closed at once, no offer) when
//! its source IP has used up its accept budget (token bucket), when the IP already has too
//! many connections that have not sent their first frame, or when the server as a whole has
//! too many of them. A connection's handshake slot is released by its first frame.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{AcceptLimits, SignalConfig};
use hbb_common::{
    rendezvous_proto::punch_hole_response::Failure::ID_NOT_EXIST,
    tokio::{self, time::Duration},
};
use std::net::IpAddr;
use support::{
    Conn, Recv, Rendezvous, TcpClient, TestServer, expect_msg, next_client_ip, read_offer,
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

/// Asserts the server closes a fresh connection from `ip` without sending anything.
async fn assert_refused(server: &TestServer, ip: IpAddr) {
    let mut conn = TcpClient::connect_from(ip, server.main_addr()).await;
    let got = conn.recv_within(WAIT).await;
    assert!(
        matches!(got, Recv::Closed),
        "expected a refusal, got {got:?}"
    );
}

/// The first frame of `conn` (a plain punch hole request) is served.
async fn first_frame(conn: &mut TcpClient, server: &TestServer) {
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

#[tokio::test]
async fn the_server_refuses_new_handshakes_beyond_its_global_cap() {
    // Arrange
    let server = server_with(AcceptLimits {
        pending: 2,
        ..AcceptLimits::default()
    })
    .await;
    let mut first = admitted(&server, next_client_ip()).await;
    let _second = admitted(&server, next_client_ip()).await;

    // Act + Assert
    assert_refused(&server, next_client_ip()).await;
    first_frame(&mut first, &server).await;
    admitted(&server, next_client_ip()).await;
    server.stop().await;
}

#[tokio::test]
async fn a_closed_pending_connection_gives_its_slot_back() {
    // Arrange
    let server = server_with(AcceptLimits {
        pending: 1,
        ..AcceptLimits::default()
    })
    .await;
    let first = admitted(&server, next_client_ip()).await;
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
        let mut conn = admitted(&server, ip).await;
        first_frame(&mut conn, &server).await;
    }

    // Act + Assert
    assert_refused(&server, ip).await;
    admitted(&server, next_client_ip()).await;
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
