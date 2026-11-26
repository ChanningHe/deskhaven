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
    Conn, Rendezvous, STEP, TcpClient, TestServer, UdpClient, addr, allow_all, config,
    free_base_port, memory_store, ports_free, rv, wait_until_ready,
};
use tokio_util::sync::CancellationToken;

/// The server's per-lookup timeout is 3 s.
const DNS_STEP: Duration = Duration::from_secs(10);
/// Listening must not wait for relay DNS (well under the 3 s resolve timeout).
const LISTEN_PROMPTLY: Duration = Duration::from_secs(2);
/// Tries on fresh free ports before a lost port race fails the test.
const START_ATTEMPTS: usize = 10;

#[tokio::test]
async fn handle_reports_serving_while_listeners_are_bound() {
    for _ in 0..START_ATTEMPTS {
        let port = free_base_port();
        let server = SignalServer::new(config(port), memory_store(), allow_all());
        let handle = server.handle();
        assert!(!handle.is_serving(), "not before run");
        let shutdown = CancellationToken::new();

        let task = tokio::spawn(server.run(shutdown.clone()));
        if timeout(STEP, handle.serving()).await.unwrap().is_err() {
            let res = task.await.unwrap();
            assert!(matches!(res, Err(SignalError::Bind { .. })), "{res:?}");
            continue;
        }

        assert!(handle.is_serving());
        assert!(
            wait_until_ready(port, &task).await,
            "listening while serving"
        );
        shutdown.cancel();
        assert!(matches!(timeout(STEP, task).await, Ok(Ok(Ok(())))));
        assert!(!handle.is_serving(), "not after run");
        assert!(matches!(handle.serving().await, Err(SignalError::Stopped)));
        return;
    }
    panic!("could not start the signal server");
}

async fn test_nat(port: u16) -> i32 {
    TcpClient::connect(addr(port)).await.test_nat().await.port
}

async fn register_peer_udp(port: u16) -> bool {
    let mut client = UdpClient::bind(addr(port)).await;
    client.register_peer("m103-udp-test").await.request_pk
}

#[tokio::test]
async fn serves_ports_then_cancel_returns_ok_stops_tasks_and_releases_ports() {
    let server = TestServer::start().await;
    let port = server.port;
    // An idle connection: upstream keeps its task alive for 30 s waiting for a message.
    let mut idle = TcpStream::connect(addr(port)).await.unwrap();

    let local_port = test_nat(port).await;
    assert!(local_port > 0, "main TCP port answers TestNatRequest");
    assert!(test_nat(port - 1).await > 0, "NAT test port answers");
    TcpStream::connect(addr(port + 2)).await.unwrap();
    assert!(
        register_peer_udp(port).await,
        "unknown peer is asked for its pk"
    );

    server.stop().await;
    assert!(ports_free(port), "all ports are released");
    // The server's KeyExchange offer is still unread; the stream must end (or be reset) right
    // after it.
    let mut rest = Vec::new();
    let _end_or_reset = timeout(STEP, idle.read_to_end(&mut rest))
        .await
        .expect("connection task ended with run");
}

/// Upstream's ConfigureUpdate from loopback was an unauthenticated local backdoor.
#[tokio::test]
async fn loopback_configure_update_has_no_effect() {
    let server = TestServer::start().await;
    let mut client = server.udp().await;
    let attack = rv(|m| {
        m.set_configure_update(ConfigUpdate {
            serial: 99,
            rendezvous_servers: vec!["10.0.0.66:21116".to_owned()],
            ..Default::default()
        })
    });

    client.send(&attack).await;
    let answer = client.register_peer("m103-udp-test").await;

    assert!(answer.request_pk);
    client.assert_silent().await;
    let nat = server.tcp().await.test_nat().await;
    assert!(nat.cu.is_none(), "TestNatResponse carries no config");
    server.stop().await;
}

#[tokio::test]
async fn unresolvable_initial_relay_does_not_delay_listening() {
    let port = free_base_port();
    let cfg = SignalConfig {
        relay_servers: vec!["relay.invalid".to_owned(), "192.0.2.1".to_owned()],
        ..config(port)
    };
    let shutdown = CancellationToken::new();
    let started = tokio::time::Instant::now();

    let task = tokio::spawn(run(cfg, memory_store(), allow_all(), shutdown.clone()));
    let ready = wait_until_ready(port, &task).await;

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

#[tokio::test]
async fn relay_update_rejects_unresolvable_entries_and_keeps_serving() {
    let server = TestServer::start().await;
    let port = server.port;
    let handle = server.handle.clone();

    // An unresolvable name is being looked up while a client talks to the server.
    let update = tokio::spawn(async move {
        handle
            .set_relay_servers(vec!["10.0.0.1".to_owned(), "relay.invalid".to_owned()])
            .await
    });
    let answered = register_peer_udp(port).await;
    let res = timeout(DNS_STEP, update).await.unwrap().unwrap();

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
        let res = run(
            config(port),
            memory_store(),
            allow_all(),
            CancellationToken::new(),
        )
        .await;
        assert!(
            matches!(res, Err(SignalError::InvalidPort(p)) if p == port),
            "port {port}: {res:?}"
        );
    }
}

#[tokio::test]
async fn port_in_use_fails_with_bind_error() {
    // Hold the main TCP port without SO_REUSEPORT.
    let port = free_base_port();
    let _taken = std::net::TcpListener::bind(addr(port)).unwrap();

    let res = timeout(
        STEP,
        run(
            config(port),
            memory_store(),
            allow_all(),
            CancellationToken::new(),
        ),
    )
    .await
    .expect("bind failure is reported promptly");

    match res {
        Err(err @ SignalError::Bind { port: p, .. }) => {
            assert_eq!(p, port);
            assert!(err.to_string().contains(&port.to_string()), "{err}");
        }
        other => panic!("expected Bind error, got {other:?}"),
    }
}

fn is_addr_in_use(err: &SignalError) -> bool {
    std::error::Error::source(err)
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .is_some_and(|io| io.kind() == std::io::ErrorKind::AddrInUse)
}

/// Upstream bound with SO_REUSEPORT, so a second instance on the same ports silently split the
/// clients with the first.
#[tokio::test]
async fn second_server_on_the_same_ports_fails_with_addr_in_use() {
    let first = TestServer::start().await;

    let res = timeout(
        STEP,
        run(
            config(first.port),
            memory_store(),
            allow_all(),
            CancellationToken::new(),
        ),
    )
    .await
    .expect("bind failure is reported promptly");

    match &res {
        Err(err @ SignalError::Bind { port, .. }) => {
            assert_eq!(*port, first.port);
            assert!(is_addr_in_use(err), "{err:?}");
        }
        other => panic!("expected Bind error, got {other:?}"),
    }
    assert!(first.tcp().await.test_nat().await.port > 0, "first serves");
    assert!(register_peer_udp(first.port).await, "first owns UDP");
    first.stop().await;
}

/// A stopped server's connections linger in TIME_WAIT / FIN_WAIT; SO_REUSEADDR lets it start
/// again on the same ports at once.
#[tokio::test]
async fn restart_right_after_stop_with_open_connections_binds_at_once() {
    let server = TestServer::start().await;
    let port = server.port;
    let mut main = server.tcp().await;
    let mut nat = server.nat().await;
    let mut ws = server.ws().await;
    for conn in [&mut main, &mut nat] {
        assert!(conn.test_nat().await.port > 0);
    }
    ws.test_nat().await;
    server.stop().await;
    drop((main, nat, ws));

    let shutdown = CancellationToken::new();
    let task = tokio::spawn(run(
        config(port),
        memory_store(),
        allow_all(),
        shutdown.clone(),
    ));

    assert!(
        wait_until_ready(port, &task).await,
        "rebind failed: {:?}",
        task.await
    );
    assert!(register_peer_udp(port).await);
    shutdown.cancel();
    let res = timeout(STEP, task).await.expect("run returns promptly");
    assert!(matches!(res, Ok(Ok(()))), "{res:?}");
}

#[tokio::test]
async fn handle_reports_stopped_once_run_has_returned() {
    let server = SignalServer::new(config(free_base_port()), memory_store(), allow_all());
    let handle = server.handle();
    let shutdown = CancellationToken::new();
    shutdown.cancel();

    let res = timeout(STEP, server.run(shutdown)).await.unwrap();

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
