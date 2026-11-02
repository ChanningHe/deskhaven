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

/// RegisterPeer carries no proof of ownership (no uuid); over a stream it would let anyone
/// redirect a victim's deliveries (review HIGH-1: a websocket client claiming a loopback IP
/// skips the IP-change check). It is answered `request_pk` (prove it with RegisterPk) and
/// changes nothing: the victim, registered over UDP, keeps receiving its punch holes, and the
/// attacker's connections get none.
#[tokio::test]
async fn register_peer_over_a_stream_never_binds_or_moves_the_peer() {
    const ID: &str = "victim-peer";
    // Arrange
    let server = TestServer::start().await;
    let (mut victim, _) = server.online_peer(ID).await;
    let mut tcp = server.tcp().await;
    let mut ws = support::WsClient::connect_from(
        support::next_client_ip(),
        server.ws_addr(),
        &[("X-Real-IP", "127.0.0.1")],
    )
    .await;

    // Act
    let tcp_res = tcp.register_peer(ID).await;
    let ws_res = ws.register_peer(ID).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;

    // Assert
    assert!(tcp_res.request_pk && ws_res.request_pk);
    let ph = expect_msg!(victim.recv().await, PunchHole);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    tcp.assert_silent().await;
    ws.assert_silent().await;
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
        match target.recv_within(support::QUIET).await {
            support::Recv::Quiet => {}
            other => panic!("after {:?}: {other:?}", started.elapsed()),
        }
    }

    // Assert
    assert!(target.heartbeats() >= 4, "{}", target.heartbeats());
    assert!(is_online(&server, ID).await);
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    expect_msg!(target.recv().await, PunchHole);
    server.stop().await;
}

/// A registered peer that stops answering heartbeats is dropped after the idle timeout (its
/// socket reads end of stream once the queued heartbeats are drained; offline alone would also
/// follow from REG_TIMEOUT) and is offline. Paused clock.
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
    match target.recv_within(STEP).await {
        support::Recv::Closed => {}
        other => panic!("connection not dropped: {other:?}"),
    }
    assert!(target.heartbeats() > 0, "heartbeats were sent");
    assert!(!is_online(&server, ID).await);
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(OFFLINE));
    server.stop().await;
}

/// The server's bound on one send to a stream connection (`STREAM_SEND_TIMEOUT`).
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// RequestRelays sent at a peer that does not read, in batches (each forwarded one is about 400
/// bytes): enough, repeated, to fill its receive window and the server's send buffer.
const FLOOD_BATCH: usize = 500;
/// Virtual time that passes after each batch. The server's built-in self test (`test_hbbs`)
/// pings its own UDP port on a virtual 1 s tick but gives up after 12 s of *real* time without
/// a reply, and the tick only runs when the paused clock moves: one long flood on a loaded
/// machine (a single-threaded runtime shared with the server) between two moves ends the server
/// with `SelfTest(Timeout of test_hbbs)`. So the flood is short batches with the clock moving
/// in between, and it stops as soon as the peer is dropped instead of being sized for the
/// worst case.
const BATCH_TICK: Duration = Duration::from_millis(500);

/// M1-08 review (M2): a registered peer that stops reading must not pin its connection task.
/// Upstream-style unbounded sends blocked the task once the socket buffers were full (so the
/// idle timeout never fired); now each send is bounded, the connection ends and the peer is
/// offline within SEND_TIMEOUT of the stall, well before REG_TIMEOUT would mark it offline.
/// Paused clock.
#[tokio::test(start_paused = true)]
async fn registered_peer_that_never_reads_is_dropped_by_the_send_timeout() {
    const ID: &str = "tcp-peer-wedged";
    // Arrange
    let server = TestServer::start().await;
    let mut target = support::TcpClient::connect_small_buffer(server.main_addr()).await;
    assert_eq!(target.register_pk(ID, &PeerKeys::random()).await, OK);
    let registered = Instant::now();
    let req = support::max_size_request_relay(ID, &server.licence_key);

    // Act: the peer never reads again while RequestRelays for it keep coming.
    let mut controller = server.tcp().await;
    while is_online(&server, ID).await {
        assert!(
            registered.elapsed() < REG_TIMEOUT,
            "still bound {:?} after registration",
            registered.elapsed()
        );
        for _ in 0..FLOOD_BATCH {
            controller.request_relay(req.clone()).await;
        }
        // Once the peer is gone each further RequestRelay is answered "offline"; read those so
        // the controller's own socket never backs up.
        while controller.try_recv(Duration::ZERO).await.is_some() {}
        tokio::time::advance(BATCH_TICK).await;
    }

    // Assert: gone by the send timeout (a send only gives up after SEND_TIMEOUT), not by
    // REG_TIMEOUT running out.
    let elapsed = registered.elapsed();
    assert!(
        elapsed >= SEND_TIMEOUT,
        "dropped after {elapsed:?}, before a send could have timed out"
    );
    assert!(
        elapsed < REG_TIMEOUT,
        "inconclusive: {elapsed:?} since registration"
    );
    server.stop().await;
}
