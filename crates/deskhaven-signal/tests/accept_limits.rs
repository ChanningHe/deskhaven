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
const QUIET: Duration = Duration::from_millis(300);

async fn server_with(limits: AcceptLimits) -> TestServer {
    TestServer::start_with(|cfg| SignalConfig {
        accept_limits: limits,
        ..cfg
    })
    .await
}

async fn admitted(server: &TestServer, ip: IpAddr) -> TcpClient {
    let mut conn = TcpClient::connect_from(ip, server.main_addr()).await;
    read_offer(&mut conn).await;
    conn
}

async fn established(server: &TestServer, ip: IpAddr) -> TcpClient {
    let mut conn = admitted(server, ip).await;
    first_frame(&mut conn, server).await;
    conn
}

async fn silent_ws(server: &TestServer, ip: IpAddr) -> TcpStream {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::new(ip, 0)).unwrap();
    socket.connect(server.ws_addr()).await.unwrap()
}

async fn assert_closed(conn: &mut impl Conn) {
    let got = conn.recv_within(WAIT).await;
    assert!(matches!(got, Recv::Closed), "expected a close, got {got:?}");
}

async fn assert_open(conn: &mut impl Conn) {
    let got = conn.recv_within(QUIET).await;
    assert!(
        matches!(got, Recv::Quiet),
        "expected an open connection, got {got:?}"
    );
}

async fn assert_refused(server: &TestServer, ip: IpAddr) {
    assert_closed(&mut TcpClient::connect_from(ip, server.main_addr()).await).await;
}

async fn first_frame(conn: &mut impl Rendezvous, server: &TestServer) {
    conn.punch_hole_request(server.punch_req("no-such-peer"))
        .await;
    let res = expect_msg!(conn.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
}

#[tokio::test]
async fn an_ip_with_too_many_pending_handshakes_is_refused_until_one_sends_a_frame() {
    let server = server_with(AcceptLimits {
        per_ip_pending: 2,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    let mut first = admitted(&server, ip).await;
    let _second = admitted(&server, ip).await;

    assert_refused(&server, ip).await;
    admitted(&server, next_client_ip()).await;
    first_frame(&mut first, &server).await;
    admitted(&server, ip).await;
    server.stop().await;
}

#[tokio::test]
async fn silent_connections_at_the_global_cap_cannot_keep_a_fresh_source_out() {
    let server = server_with(AcceptLimits {
        pending: 4,
        ..AcceptLimits::default()
    })
    .await;
    let mut silent = Vec::new();
    for _ in 0..4 {
        silent.push(admitted(&server, next_client_ip()).await);
    }

    let _fresh = established(&server, next_client_ip()).await;

    assert_closed(&mut silent[0]).await;
    for conn in &mut silent[1..] {
        assert_open(conn).await;
    }
    server.stop().await;
}

#[tokio::test]
async fn an_ip_that_used_up_its_accept_burst_is_refused() {
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

    assert_refused(&server, ip).await;
    admitted(&server, next_client_ip()).await;
    server.stop().await;
}

#[tokio::test]
async fn an_ip_with_too_many_established_connections_is_refused() {
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

    assert_refused(&server, ip).await;
    established(&server, next_client_ip()).await;
    server.stop().await;
}

#[tokio::test]
async fn established_keep_alive_connections_at_the_global_cap_cannot_refuse_a_fresh_client() {
    let server = server_with(AcceptLimits {
        connections: 4,
        ..AcceptLimits::default()
    })
    .await;
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(established(&server, next_client_ip()).await);
    }

    let _fresh = established(&server, next_client_ip()).await;

    assert_closed(&mut held[0]).await;
    for conn in &mut held[1..] {
        assert_open(conn).await;
    }
    server.stop().await;
}

#[tokio::test]
async fn registered_online_peers_survive_eviction_while_idle_unregistered_ones_go() {
    // The registered peer is the oldest connection.
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

    let _fresh = established(&server, next_client_ip()).await;

    assert_closed(&mut idle).await;
    assert_open(&mut peer).await;
    assert_open(&mut recent).await;
    server.stop().await;
}

#[tokio::test]
async fn silent_websocket_connections_from_one_source_cannot_refuse_a_fresh_main_port_client() {
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

    established(&server, next_client_ip()).await;
    established(&server, next_client_ip()).await;
    server.stop().await;
}

#[tokio::test]
async fn websocket_connections_are_capped_per_source_after_the_handshake() {
    let server = server_with(AcceptLimits {
        per_ip_connections: 1,
        ..AcceptLimits::default()
    })
    .await;
    let ip = next_client_ip();
    let mut first = WsClient::connect_from(ip, server.ws_addr(), &[]).await;
    first_frame(&mut first, &server).await;

    assert_closed(&mut WsClient::connect_from(ip, server.ws_addr(), &[]).await).await;
    let mut other = WsClient::connect_from(next_client_ip(), server.ws_addr(), &[]).await;
    first_frame(&mut other, &server).await;
    server.stop().await;
}

#[tokio::test]
async fn the_default_limits_admit_a_burst_of_connections_from_one_ip() {
    // A NAT gateway in front of many devices reconnecting after a server restart.
    let server = TestServer::start().await;
    let ip = next_client_ip();

    let mut held = Vec::new();
    for _ in 0..32 {
        held.push(admitted(&server, ip).await);
    }
    server.stop().await;
}
