//! Acceptance tests (M1-03): the signal server runs as a library on the caller's runtime,
//! serves its TCP / UDP / websocket ports, and on cancellation returns `Ok`, stops every
//! connection task and releases every port.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use deskhaven_signal::{SignalConfig, SignalError, SignalServer, run};
use hbb_common::{
    protobuf::Message as _,
    rendezvous_proto::{RegisterPeer, RendezvousMessage, TestNatRequest, rendezvous_message},
    tcp::FramedStream,
    tokio::{
        self,
        io::AsyncReadExt,
        net::{TcpStream, UdpSocket},
        task::JoinHandle,
        time::{Duration, sleep, timeout},
    },
};
use sodiumoxide::crypto::sign;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio_util::sync::CancellationToken;

const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const STEP: Duration = Duration::from_secs(5);
const STEP_MS: u64 = 5_000;
const READY_POLL: Duration = Duration::from_millis(20);
const START_ATTEMPTS: usize = 10;

fn config(port: u16) -> SignalConfig {
    SignalConfig {
        bind: Some(LOCALHOST),
        port,
        ..SignalConfig::new(sign::gen_keypair().1)
    }
}

fn addr(port: u16) -> SocketAddr {
    SocketAddr::new(LOCALHOST, port)
}

/// The server derives two more TCP ports from its main port: NAT test on port-1 and
/// websocket on port+2.
fn ports_free(port: u16) -> bool {
    std::net::TcpListener::bind(addr(port - 1)).is_ok()
        && std::net::TcpListener::bind(addr(port)).is_ok()
        && std::net::TcpListener::bind(addr(port + 2)).is_ok()
        && std::net::UdpSocket::bind(addr(port)).is_ok()
}

/// A main port whose derived ports are all free right now (the caller still has to cope
/// with losing a race for them).
fn free_base_port() -> u16 {
    for _ in 0..100 {
        let port = std::net::TcpListener::bind(addr(0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        if (3..=65533).contains(&port) && ports_free(port) {
            return port;
        }
    }
    panic!("no free port triple found");
}

struct Running {
    port: u16,
    shutdown: CancellationToken,
    task: JoinHandle<Result<(), SignalError>>,
}

async fn all_tcp_ports_accept(port: u16) -> bool {
    for p in [port - 1, port, port + 2] {
        if TcpStream::connect(addr(p)).await.is_err() {
            return false;
        }
    }
    true
}

/// `true` once every TCP port accepts, `false` if `run` ended first (a lost port race).
async fn wait_until_ready(port: u16, task: &JoinHandle<Result<(), SignalError>>) -> bool {
    let deadline = tokio::time::Instant::now() + STEP;
    while tokio::time::Instant::now() < deadline {
        if task.is_finished() {
            return false;
        }
        if all_tcp_ports_accept(port).await {
            return true;
        }
        sleep(READY_POLL).await;
    }
    panic!("signal server on port {port} did not become ready");
}

async fn start() -> Running {
    for _ in 0..START_ATTEMPTS {
        let port = free_base_port();
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(run(config(port), shutdown.clone()));
        if wait_until_ready(port, &task).await {
            return Running {
                port,
                shutdown,
                task,
            };
        }
        let res = task.await.unwrap();
        assert!(
            matches!(res, Err(SignalError::Bind { .. })),
            "unexpected early exit: {res:?}"
        );
    }
    panic!("could not start the signal server");
}

fn test_nat_request() -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    msg.set_test_nat_request(TestNatRequest::default());
    msg
}

/// Sends a TestNatRequest over TCP and returns the port the server saw us on.
async fn test_nat(port: u16) -> i32 {
    let mut stream = FramedStream::new(addr(port), None, STEP_MS).await.unwrap();
    stream.send(&test_nat_request()).await.unwrap();
    let bytes = stream.next_timeout(STEP_MS).await.unwrap().unwrap();
    match RendezvousMessage::parse_from_bytes(&bytes).unwrap().union {
        Some(rendezvous_message::Union::TestNatResponse(res)) => res.port,
        other => panic!("expected TestNatResponse, got {other:?}"),
    }
}

/// Sends a RegisterPeer datagram and returns the server's `request_pk` answer.
async fn register_peer_udp(port: u16) -> bool {
    let socket = UdpSocket::bind(addr(0)).await.unwrap();
    let mut msg = RendezvousMessage::new();
    msg.set_register_peer(RegisterPeer {
        id: "m103-udp-test".to_owned(),
        ..Default::default()
    });
    socket
        .send_to(&msg.write_to_bytes().unwrap(), addr(port))
        .await
        .unwrap();
    let mut buf = [0u8; 1024];
    let (n, _) = timeout(STEP, socket.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    match RendezvousMessage::parse_from_bytes(&buf[..n])
        .unwrap()
        .union
    {
        Some(rendezvous_message::Union::RegisterPeerResponse(res)) => res.request_pk,
        other => panic!("expected RegisterPeerResponse, got {other:?}"),
    }
}

#[tokio::test]
async fn serves_ports_then_cancel_returns_ok_stops_tasks_and_releases_ports() {
    // Arrange
    let server = start().await;
    let port = server.port;
    // An idle connection: upstream keeps its task alive for 30 s waiting for a message.
    let mut idle = TcpStream::connect(addr(port)).await.unwrap();

    // Act + Assert: every port answers.
    let local_port = test_nat(port).await;
    assert!(local_port > 0, "main TCP port answers TestNatRequest");
    assert!(test_nat(port - 1).await > 0, "NAT test port answers");
    TcpStream::connect(addr(port + 2)).await.unwrap();
    assert!(
        register_peer_udp(port).await,
        "unknown peer is asked for its pk"
    );

    // Act: cancel.
    server.shutdown.cancel();
    let res = timeout(STEP, server.task)
        .await
        .expect("run returns promptly");

    // Assert
    assert!(matches!(res, Ok(Ok(()))), "run returns Ok: {res:?}");
    assert!(ports_free(port), "all ports are released");
    let mut buf = [0u8; 16];
    let read = timeout(STEP, idle.read(&mut buf))
        .await
        .expect("idle conn closed");
    assert!(
        matches!(read, Ok(0)) || read.is_err(),
        "connection task ended with run"
    );
}

#[tokio::test]
async fn rejects_ports_without_room_for_nat_and_ws_ports() {
    for port in [0, 1, 2, 65534, 65535] {
        let res = run(config(port), CancellationToken::new()).await;
        assert!(
            matches!(res, Err(SignalError::InvalidPort(p)) if p == port),
            "port {port}: {res:?}"
        );
    }
}

#[tokio::test]
async fn port_in_use_fails_with_bind_error() {
    // Arrange: hold the main TCP port without SO_REUSEPORT.
    let port = free_base_port();
    let _taken = std::net::TcpListener::bind(addr(port)).unwrap();

    // Act
    let res = timeout(STEP, run(config(port), CancellationToken::new()))
        .await
        .expect("bind failure is reported promptly");

    // Assert
    match res {
        Err(err @ SignalError::Bind { port: p, .. }) => {
            assert_eq!(p, port);
            assert!(err.to_string().contains(&port.to_string()), "{err}");
        }
        other => panic!("expected Bind error, got {other:?}"),
    }
}

#[tokio::test]
async fn handle_reports_stopped_once_run_has_returned() {
    // Arrange
    let server = SignalServer::new(config(free_base_port()));
    let handle = server.handle();
    let shutdown = CancellationToken::new();
    shutdown.cancel();

    // Act
    let res = timeout(STEP, server.run(shutdown)).await.unwrap();

    // Assert
    assert!(res.is_ok(), "{res:?}");
    assert!(matches!(
        handle.set_relay_servers(vec!["10.0.0.1".to_owned()]),
        Err(SignalError::Stopped)
    ));
    handle.set_always_use_relay(true); // never fails
}
