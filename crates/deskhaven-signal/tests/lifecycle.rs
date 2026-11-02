//! Acceptance tests (M1-03): the signal server runs as a library on the caller's runtime,
//! serves its TCP / UDP / websocket ports, and on cancellation returns `Ok`, stops every
//! connection task and releases every port.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use deskhaven_signal::{SignalConfig, SignalError, SignalHandle, SignalServer, run};
use hbb_common::{
    protobuf::Message as _,
    rendezvous_proto::{
        ConfigUpdate, RegisterPeer, RendezvousMessage, TestNatRequest, rendezvous_message,
    },
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
/// How long to wait for a datagram that must NOT arrive.
const QUIET: Duration = Duration::from_millis(500);
/// Upper bound for an `.invalid` lookup to fail (the server's per-lookup timeout is 3 s).
const DNS_STEP: Duration = Duration::from_secs(10);
/// Listening must not wait for relay DNS (well under the 3 s resolve timeout).
const LISTEN_PROMPTLY: Duration = Duration::from_secs(2);

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
    handle: SignalHandle,
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
        let server = SignalServer::new(config(port));
        let handle = server.handle();
        let task = tokio::spawn(server.run(shutdown.clone()));
        if wait_until_ready(port, &task).await {
            return Running {
                port,
                handle,
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

async fn stop(server: Running) {
    server.shutdown.cancel();
    let res = timeout(STEP, server.task)
        .await
        .expect("run returns promptly");
    assert!(matches!(res, Ok(Ok(()))), "run returns Ok: {res:?}");
}

fn test_nat_request() -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    msg.set_test_nat_request(TestNatRequest::default());
    msg
}

/// Sends a TestNatRequest over TCP and returns the server's answer.
async fn test_nat_response(port: u16) -> hbb_common::rendezvous_proto::TestNatResponse {
    let mut stream = FramedStream::new(addr(port), None, STEP_MS).await.unwrap();
    stream.send(&test_nat_request()).await.unwrap();
    let bytes = stream.next_timeout(STEP_MS).await.unwrap().unwrap();
    match RendezvousMessage::parse_from_bytes(&bytes).unwrap().union {
        Some(rendezvous_message::Union::TestNatResponse(res)) => res,
        other => panic!("expected TestNatResponse, got {other:?}"),
    }
}

/// Sends a TestNatRequest over TCP and returns the port the server saw us on.
async fn test_nat(port: u16) -> i32 {
    test_nat_response(port).await.port
}

async fn send_udp(socket: &UdpSocket, port: u16, msg: &RendezvousMessage) {
    socket
        .send_to(&msg.write_to_bytes().unwrap(), addr(port))
        .await
        .unwrap();
}

/// The next datagram within `wait`, if any.
async fn recv_udp(socket: &UdpSocket, wait: Duration) -> Option<RendezvousMessage> {
    let mut buf = [0u8; 1024];
    let (n, _) = timeout(wait, socket.recv_from(&mut buf))
        .await
        .ok()?
        .unwrap();
    Some(RendezvousMessage::parse_from_bytes(&buf[..n]).unwrap())
}

fn register_peer(serial: i32) -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    msg.set_register_peer(RegisterPeer {
        id: "m103-udp-test".to_owned(),
        serial,
        ..Default::default()
    });
    msg
}

/// Sends a RegisterPeer datagram and returns the server's `request_pk` answer.
async fn register_peer_udp(port: u16) -> bool {
    let socket = UdpSocket::bind(addr(0)).await.unwrap();
    send_udp(&socket, port, &register_peer(0)).await;
    match recv_udp(&socket, STEP).await.map(|m| m.union) {
        Some(Some(rendezvous_message::Union::RegisterPeerResponse(res))) => res.request_pk,
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

    // Act + Assert: cancel.
    stop(server).await;
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

/// Upstream accepted a ConfigureUpdate datagram from loopback and then pushed its serial and
/// rendezvous servers to every client (RegisterPeer, TestNatResponse): an unauthenticated
/// local backdoor like the removed console.
#[tokio::test]
async fn loopback_configure_update_has_no_effect() {
    // Arrange
    let server = start().await;
    let port = server.port;
    let socket = UdpSocket::bind(addr(0)).await.unwrap();
    let mut attack = RendezvousMessage::new();
    attack.set_configure_update(ConfigUpdate {
        serial: 99,
        rendezvous_servers: vec!["10.0.0.66:21116".to_owned()],
        ..Default::default()
    });

    // Act
    send_udp(&socket, port, &attack).await;
    send_udp(&socket, port, &register_peer(0)).await;

    // Assert: exactly one RegisterPeerResponse, no ConfigureUpdate pushed.
    let first = recv_udp(&socket, STEP).await.map(|m| m.union);
    assert!(
        matches!(
            first,
            Some(Some(rendezvous_message::Union::RegisterPeerResponse(_)))
        ),
        "{first:?}"
    );
    let extra = recv_udp(&socket, QUIET).await;
    assert!(extra.is_none(), "unexpected datagram: {extra:?}");
    assert!(
        test_nat_response(port).await.cu.is_none(),
        "TestNatResponse carries no config"
    );
    stop(server).await;
}

/// Startup binds every listener before resolving the initial relay list, so an unresolvable
/// relay cannot delay listening (it is retried on the timer).
#[tokio::test]
async fn unresolvable_initial_relay_does_not_delay_listening() {
    // Arrange
    let port = free_base_port();
    let cfg = SignalConfig {
        relay_servers: vec!["relay.invalid".to_owned(), "192.0.2.1".to_owned()],
        ..config(port)
    };
    let shutdown = CancellationToken::new();
    let started = tokio::time::Instant::now();

    // Act
    let task = tokio::spawn(run(cfg, shutdown.clone()));
    let ready = wait_until_ready(port, &task).await;

    // Assert
    assert!(ready, "run ended early: {:?}", task.await);
    assert!(
        started.elapsed() < LISTEN_PROMPTLY,
        "{:?}",
        started.elapsed()
    );
    assert!(register_peer_udp(port).await);
    shutdown.cancel();
    let res = timeout(STEP, task).await.expect("run returns promptly");
    assert!(matches!(res, Ok(Ok(()))), "{res:?}");
}

/// The handle resolves on the caller's task: all-or-nothing, the rejected entries are named,
/// and the server keeps serving meanwhile.
#[tokio::test]
async fn relay_update_rejects_unresolvable_entries_and_keeps_serving() {
    // Arrange
    let server = start().await;
    let port = server.port;
    let handle = server.handle.clone();

    // Act: an unresolvable name is being looked up while a client talks to the server.
    let update = tokio::spawn(async move {
        handle
            .set_relay_servers(vec!["10.0.0.1".to_owned(), "relay.invalid".to_owned()])
            .await
    });
    let answered = register_peer_udp(port).await;
    let res = timeout(DNS_STEP, update).await.unwrap().unwrap();

    // Assert
    assert!(answered, "the server loop keeps serving");
    match res {
        Err(SignalError::RelayServersRejected { rejected }) => {
            assert_eq!(rejected, ["relay.invalid"]);
        }
        other => panic!("expected RelayServersRejected, got {other:?}"),
    }
    let accepted = server
        .handle
        .set_relay_servers(vec!["10.0.0.1".to_owned(), String::new()])
        .await
        .unwrap();
    assert_eq!(accepted, ["10.0.0.1"], "empty entries are dropped");
    assert!(register_peer_udp(port).await);
    stop(server).await;
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
        handle.set_relay_servers(vec!["10.0.0.1".to_owned()]).await,
        Err(SignalError::Stopped)
    ));
    assert!(matches!(
        handle.set_always_use_relay(true),
        Err(SignalError::Stopped)
    ));
}
