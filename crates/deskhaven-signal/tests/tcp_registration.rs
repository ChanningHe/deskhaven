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
    online_bits, punch_hole_sent, register_peer, register_pk, request_relay, rv,
};

const POLL: Duration = Duration::from_millis(20);

async fn is_online(server: &TestServer, id: &str) -> bool {
    let res = server.nat().await.online_request(&[id]).await;
    online_bits(&res.states, 1)[0]
}

async fn wait_online(server: &TestServer, id: &str, online: bool) {
    let deadline = Instant::now() + STEP;
    while is_online(server, id).await != online {
        assert!(Instant::now() < deadline, "{id}: online != {online}");
        sleep(POLL).await;
    }
}

#[tokio::test]
async fn peer_registered_over_tcp_receives_punch_holes_on_its_connection() {
    const ID: &str = "tcp-peer-punch";
    let server = TestServer::start().await;
    let mut target = server.tcp().await;

    let res = target.register_pk(ID, &PeerKeys::random()).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut answer = server.tcp().await;
    let sent = punch_hole_sent(&ph.socket_addr, ID, "");
    answer.send(&rv(|m| m.set_punch_hole_sent(sent))).await;

    assert_eq!(res, OK);
    assert!(is_online(&server, ID).await);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(decode_addr(&res.socket_addr), answer.local_addr());
    server.stop().await;
}

#[tokio::test]
async fn peer_registered_over_websocket_receives_relay_requests_on_its_connection() {
    const ID: &str = "ws-peer-relay";
    let server = TestServer::start().await;
    let mut target = server.ws().await;

    let res = target.register_pk(ID, &PeerKeys::random()).await;
    let mut controller = server.tcp().await;
    controller
        .request_relay(request_relay(ID, &server.licence_key))
        .await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);

    assert_eq!(res, OK);
    assert_eq!(decode_addr(&fwd.socket_addr), controller.local_addr());
    assert!(fwd.token.is_empty() && fwd.licence_key.is_empty());
    assert!(is_online(&server, ID).await);
    server.stop().await;
}

#[tokio::test]
async fn register_pk_over_tcp_applies_the_udp_checks() {
    const ID: &str = "tcp-peer-checked";
    let server = TestServer::start().await;
    let (_owner, _) = server.online_peer(ID).await;
    let mut conn = server.tcp().await;
    let mut malformed = server.tcp().await;
    let bad_pk = PeerKeys {
        pk: vec![1; 31].into(),
        ..PeerKeys::random()
    };

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

#[tokio::test]
async fn malformed_ids_are_refused_over_streams() {
    const BAD_ID: &str = "abc\ndef";
    const UNICODE_ID: &str = "a\u{5f20}\u{4e09}1234";
    let server = TestServer::start().await;
    let mut conn = server.tcp().await;
    let mut ws_conn = server.ws().await;
    let mut tcp_peer = server.tcp().await;
    let mut ws_peer = server.ws().await;

    assert_eq!(
        conn.register_pk(BAD_ID, &PeerKeys::random()).await,
        UUID_MISMATCH
    );
    assert_eq!(
        conn.register_pk("tcp-peer-renamed", &PeerKeys::random())
            .await,
        OK
    );
    assert_eq!(
        ws_conn.register_pk(UNICODE_ID, &PeerKeys::random()).await,
        OK
    );
    assert!(ws_conn.register_peer(UNICODE_ID).await.request_pk);
    tcp_peer.send(&register_peer("abc\u{202e}def")).await;
    tcp_peer.assert_closed().await;
    ws_peer.send(&register_peer("10\u{0}aaa")).await;
    ws_peer.assert_closed().await;
    server.stop().await;
}

#[tokio::test]
async fn register_pk_over_tcp_is_persisted() {
    const ID: &str = "tcp-peer-saved";
    let store = Arc::new(MemoryPeerStore::new());
    let server = TestServer::start_with_store(|cfg| cfg, store.clone()).await;
    let keys = PeerKeys::random();

    assert_eq!(server.tcp().await.register_pk(ID, &keys).await, OK);

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

/// RegisterPeer carries no proof of ownership, so over a stream it must not redirect a
/// victim's deliveries.
#[tokio::test]
async fn register_peer_over_a_stream_never_binds_or_moves_the_peer() {
    const ID: &str = "victim-peer";
    let server = TestServer::start().await;
    let (mut victim, _) = server.online_peer(ID).await;
    let mut tcp = server.tcp().await;
    let mut ws = support::WsClient::connect_from(
        support::next_client_ip(),
        server.ws_addr(),
        &[("X-Real-IP", "127.0.0.1")],
    )
    .await;

    let tcp_res = tcp.register_peer(ID).await;
    let ws_res = ws.register_peer(ID).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;

    assert!(tcp_res.request_pk && ws_res.request_pk);
    let ph = expect_msg!(victim.recv().await, PunchHole);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    tcp.assert_silent().await;
    ws.assert_silent().await;
    server.stop().await;
}

#[tokio::test]
async fn peer_is_offline_once_its_connection_closes() {
    const ID: &str = "tcp-peer-gone";
    let server = TestServer::start().await;
    let keys = PeerKeys::random();
    let mut target = server.tcp().await;
    assert_eq!(target.register_pk(ID, &keys).await, OK);
    let ip = target.local_addr().ip();

    drop(target);

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

#[tokio::test(start_paused = true)]
async fn heartbeats_keep_a_tcp_peer_online_past_reg_timeout() {
    const ID: &str = "tcp-peer-alive";
    let server = TestServer::start().await;
    let mut target = server.tcp().await;
    assert_eq!(target.register_pk(ID, &PeerKeys::random()).await, OK);
    let started = Instant::now();

    while started.elapsed() < REG_TIMEOUT * 2 {
        match target.recv_within(support::QUIET).await {
            support::Recv::Quiet => {}
            other => panic!("after {:?}: {other:?}", started.elapsed()),
        }
    }

    assert!(target.heartbeats() >= 4, "{}", target.heartbeats());
    assert!(is_online(&server, ID).await);
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    expect_msg!(target.recv().await, PunchHole);
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn tcp_peer_that_ignores_heartbeats_is_dropped_and_offline() {
    const ID: &str = "tcp-peer-mute";
    let server = TestServer::start().await;
    let mut target = server.tcp().await;
    assert_eq!(target.register_pk(ID, &PeerKeys::random()).await, OK);

    // Do not read, so no heartbeat is answered.
    sleep(REG_TIMEOUT + Duration::from_secs(5)).await;

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

/// Mirrors the server's `STREAM_SEND_TIMEOUT`.
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Each forwarded RequestRelay is about 400 bytes: enough, repeated, to fill the peer's receive
/// window and the server's send buffer.
const FLOOD_BATCH: usize = 500;
/// The server's UDP self-test gives up after 12 s of real time but only ticks when the paused
/// clock moves, so the flood runs in short batches with the clock moving in between.
const BATCH_TICK: Duration = Duration::from_millis(500);

#[tokio::test(start_paused = true)]
async fn registered_peer_that_never_reads_is_dropped_by_the_send_timeout() {
    const ID: &str = "tcp-peer-wedged";
    let server = TestServer::start().await;
    let mut target = support::TcpClient::connect_small_buffer(server.main_addr()).await;
    assert_eq!(target.register_pk(ID, &PeerKeys::random()).await, OK);
    let registered = Instant::now();
    let req = support::max_size_request_relay(ID, &server.licence_key);

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

    // Gone by the send timeout, not by REG_TIMEOUT running out.
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
