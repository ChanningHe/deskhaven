//! M1-08: registration over TCP and websocket. RustDesk clients in TCP mode (proxy, websocket,
//! UDP disabled; rendezvous_mediator.rs `start_tcp`) keep one connection to the main port (or
//! the websocket port) open, send only RegisterPk on it (until it is answered OK, and again
//! after UUID_MISMATCH), receive PunchHole / FetchLocalAddr / RequestRelay on it and answer
//! empty frames (server heartbeats) with empty frames. Upstream answered RegisterPk over TCP
//! with NOT_SUPPORT and ignored RegisterPeer (rs:596-604), so such clients never came online.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{MemoryPeerStore, PeerStore};
use hbb_common::{
    rendezvous_proto::{
        punch_hole_response::Failure::OFFLINE,
        register_pk_response::Result::{OK, UUID_MISMATCH},
    },
    tokio::{
        self,
        time::{Duration, Instant, sleep},
    },
};
use std::sync::Arc;
use support::{
    Conn, PeerKeys, REG_TIMEOUT, Rendezvous, STEP, TestServer, decode_addr, expect_msg,
    online_bits, punch_hole_sent, register_pk, request_relay, rv,
};

const POLL: Duration = Duration::from_millis(20);

async fn is_online(server: &TestServer, id: &str) -> bool {
    let res = server.nat().await.online_request(&[id]).await;
    online_bits(&res.states, 1)[0]
}

/// Polls `is_online` until it equals `online` (within [`STEP`]).
async fn wait_online(server: &TestServer, id: &str, online: bool) {
    let deadline = Instant::now() + STEP;
    while is_online(server, id).await != online {
        assert!(Instant::now() < deadline, "{id}: online != {online}");
        sleep(POLL).await;
    }
}

/// A peer registered over TCP is online and gets punch holes over its connection; its answer
/// (over a new connection, as clients do) reaches the controller.
#[tokio::test]
async fn peer_registered_over_tcp_receives_punch_holes_on_its_connection() {
    const ID: &str = "tcp-peer-punch";
    // Arrange
    let server = TestServer::start().await;
    let mut target = server.tcp().await;

    // Act
    let res = target.register_pk(ID, &PeerKeys::random()).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut answer = server.tcp().await;
    let sent = punch_hole_sent(&ph.socket_addr, ID, "");
    answer.send(&rv(|m| m.set_punch_hole_sent(sent))).await;

    // Assert
    assert_eq!(res, OK);
    assert!(is_online(&server, ID).await);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(decode_addr(&res.socket_addr), answer.local_addr());
    server.stop().await;
}

/// The same over websocket: RegisterPk is answered and RequestRelay reaches the peer over its
/// websocket connection (without the controller's secrets).
#[tokio::test]
async fn peer_registered_over_websocket_receives_relay_requests_on_its_connection() {
    const ID: &str = "ws-peer-relay";
    // Arrange
    let server = TestServer::start().await;
    let mut target = server.ws().await;

    // Act
    let res = target.register_pk(ID, &PeerKeys::random()).await;
    let mut controller = server.tcp().await;
    controller
        .request_relay(request_relay(ID, &server.licence_key))
        .await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);

    // Assert
    assert_eq!(res, OK);
    assert_eq!(decode_addr(&fwd.socket_addr), controller.local_addr());
    assert!(fwd.token.is_empty() && fwd.licence_key.is_empty());
    assert!(is_online(&server, ID).await);
    server.stop().await;
}

/// RegisterPk over TCP gets the UDP checks (rs:391-420): a short id or another device's uuid
/// is refused with UUID_MISMATCH, and the connection stays usable (a client answers
/// UUID_MISMATCH with a new id on the same connection); a malformed key gets no answer and
/// closes the connection.
#[tokio::test]
async fn register_pk_over_tcp_applies_the_udp_checks() {
    const ID: &str = "tcp-peer-checked";
    // Arrange
    let server = TestServer::start().await;
    let (_owner, _) = server.online_peer(ID).await;
    let mut conn = server.tcp().await;
    let mut malformed = server.tcp().await;
    let bad_pk = PeerKeys {
        pk: vec![1; 31].into(),
        ..PeerKeys::random()
    };

    // Act + Assert
    assert_eq!(
        conn.register_pk("12345", &PeerKeys::random()).await,
        UUID_MISMATCH
    );
    assert_eq!(
        conn.register_pk(ID, &PeerKeys::random()).await,
        UUID_MISMATCH
    );
    assert_eq!(
        conn.register_pk("tcp-peer-new-id", &PeerKeys::random())
            .await,
        OK
    );
    malformed.send(&register_pk("tcp-peer-bad", &bad_pk)).await;
    malformed.assert_closed().await;
    server.stop().await;
}

/// RegisterPk over TCP is persisted like over UDP.
#[tokio::test]
async fn register_pk_over_tcp_is_persisted() {
    const ID: &str = "tcp-peer-saved";
    // Arrange
    let store = Arc::new(MemoryPeerStore::new());
    let server = TestServer::start_with_store(|cfg| cfg, store.clone()).await;
    let keys = PeerKeys::random();

    // Act
    assert_eq!(server.tcp().await.register_pk(ID, &keys).await, OK);

    // Assert
    let deadline = Instant::now() + STEP;
    loop {
        if let Some(rec) = store.get(ID).await.unwrap() {
            assert_eq!(rec.uuid, keys.uuid.to_vec());
            assert_eq!(rec.pk, keys.pk.to_vec());
            break;
        }
        assert!(Instant::now() < deadline, "not saved");
        sleep(POLL).await;
    }
    server.stop().await;
}

/// RegisterPeer over TCP is answered like over UDP (rs:612-653): the pk is requested until it
/// is known.
#[tokio::test]
async fn register_peer_over_tcp_requests_the_pk_until_it_is_known() {
    const ID: &str = "tcp-peer-heartbeat";
    // Arrange
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;

    // Act + Assert
    assert!(conn.register_peer(ID).await.request_pk, "unknown id");
    assert_eq!(conn.register_pk(ID, &PeerKeys::random()).await, OK);
    assert!(!conn.register_peer(ID).await.request_pk, "pk is known");
    assert!(is_online(&server, ID).await);
    server.stop().await;
}

/// When the peer's connection closes it is offline at once (no REG_TIMEOUT wait); a new
/// registration brings it back.
#[tokio::test]
async fn peer_is_offline_once_its_connection_closes() {
    const ID: &str = "tcp-peer-gone";
    // Arrange
    let server = TestServer::start().await;
    let keys = PeerKeys::random();
    let mut target = server.tcp().await;
    assert_eq!(target.register_pk(ID, &keys).await, OK);
    let ip = target.local_addr().ip();

    // Act
    drop(target);

    // Assert
    wait_online(&server, ID, false).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(OFFLINE));
    let mut back = support::TcpClient::connect_from(ip, server.main_addr()).await;
    assert_eq!(back.register_pk(ID, &keys).await, OK);
    assert!(is_online(&server, ID).await);
    server.stop().await;
}

/// The server sends heartbeats on a registered connection; a peer that answers them stays
/// online well past REG_TIMEOUT without sending anything else (what RustDesk clients do once
/// registered), and still gets punch holes. Paused clock.
#[tokio::test(start_paused = true)]
async fn heartbeats_keep_a_tcp_peer_online_past_reg_timeout() {
    const ID: &str = "tcp-peer-alive";
    // Arrange
    let server = TestServer::start().await;
    let mut target = server.tcp().await;
    assert_eq!(target.register_pk(ID, &PeerKeys::random()).await, OK);
    let started = Instant::now();

    // Act: listen (answering heartbeats) for twice REG_TIMEOUT.
    while started.elapsed() < REG_TIMEOUT * 2 {
        target.assert_silent().await;
    }

    // Assert
    assert!(target.heartbeats() >= 4, "{}", target.heartbeats());
    assert!(is_online(&server, ID).await);
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    expect_msg!(target.recv().await, PunchHole);
    server.stop().await;
}

/// A registered peer that stops answering heartbeats is dropped after the idle timeout and is
/// offline. Paused clock.
#[tokio::test(start_paused = true)]
async fn tcp_peer_that_ignores_heartbeats_is_dropped_and_offline() {
    const ID: &str = "tcp-peer-mute";
    // Arrange
    let server = TestServer::start().await;
    let mut target = server.tcp().await;
    assert_eq!(target.register_pk(ID, &PeerKeys::random()).await, OK);

    // Act: do not read (so no heartbeat is answered) for longer than the idle timeout.
    sleep(REG_TIMEOUT + Duration::from_secs(5)).await;

    // Assert
    assert!(!is_online(&server, ID).await);
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(OFFLINE));
    server.stop().await;
}
