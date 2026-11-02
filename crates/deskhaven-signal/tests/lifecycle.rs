//! Acceptance tests (M1-03): the signal server runs as a library on the caller's runtime,
//! serves its TCP / UDP / websocket ports, and on cancellation returns `Ok`, stops every
//! connection task and releases every port.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{SignalConfig, SignalError, SignalServer, run};
use hbb_common::{
    rendezvous_proto::ConfigUpdate,
    tokio::{
        self,
        io::AsyncReadExt,
        net::TcpStream,
        time::{Duration, timeout},
    },
};
use support::{
    Conn, Rendezvous, STEP, TcpClient, TestServer, UdpClient, addr, config, free_base_port,
    ports_free, rv, wait_until_ready,
};
use tokio_util::sync::CancellationToken;

/// Upper bound for an `.invalid` lookup to fail (the server's per-lookup timeout is 3 s).
const DNS_STEP: Duration = Duration::from_secs(10);
/// Listening must not wait for relay DNS (well under the 3 s resolve timeout).
const LISTEN_PROMPTLY: Duration = Duration::from_secs(2);

/// Sends a TestNatRequest over TCP and returns the port the server saw us on.
async fn test_nat(port: u16) -> i32 {
    TcpClient::connect(addr(port)).await.test_nat().await.port
}

/// Sends a RegisterPeer datagram and returns the server's `request_pk` answer.
async fn register_peer_udp(port: u16) -> bool {
    let mut client = UdpClient::bind(addr(port)).await;
    client.register_peer("m103-udp-test").await.request_pk
}

#[tokio::test]
async fn serves_ports_then_cancel_returns_ok_stops_tasks_and_releases_ports() {
    // Arrange
    let server = TestServer::start().await;
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
    server.stop().await;
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
    let server = TestServer::start().await;
    let mut client = server.udp().await;
    let attack = rv(|m| {
        m.set_configure_update(ConfigUpdate {
            serial: 99,
            rendezvous_servers: vec!["10.0.0.66:21116".to_owned()],
            ..Default::default()
        })
    });

    // Act
    client.send(&attack).await;
    let answer = client.register_peer("m103-udp-test").await;

    // Assert: exactly one RegisterPeerResponse, no ConfigureUpdate pushed.
    assert!(answer.request_pk);
    client.assert_silent().await;
    let nat = server.tcp().await.test_nat().await;
    assert!(nat.cu.is_none(), "TestNatResponse carries no config");
    server.stop().await;
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
    let server = TestServer::start().await;
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
    server.stop().await;
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
