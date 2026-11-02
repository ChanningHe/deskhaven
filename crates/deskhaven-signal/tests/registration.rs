//! Upstream-behavior baseline (M1-04): peer registration (RegisterPeer / RegisterPk).
//!
//! Each test pins a behavior of upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs,
//! `peer.rs` = src/peer.rs at that commit; line numbers refer to it), minus the removals in
//! CHANGES.md. Later tasks must keep these green unless their task explicitly changes the
//! behavior (those tests say so in their name). Rate limits live in `rate_limits.rs`.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    rendezvous_proto::register_pk_response::Result::{NOT_SUPPORT, OK, UUID_MISMATCH},
    tokio,
};
use support::{Conn, PeerKeys, Rendezvous, TestServer, expect_msg, register_peer, register_pk};

const ID: &str = "123456789";
/// peer.rs:27 `MAX_PEER_ID_LEN`.
const MAX_ID_LEN: usize = 253;
/// rs:396: RegisterPk ids shorter than this are refused.
const MIN_PK_ID_LEN: usize = 6;

/// rs:612-653 `update_addr`: an id without a stored pk is asked for it (`request_pk`), and a
/// RegisterPeer alone stores nothing; after RegisterPk it only refreshes the address. Address
/// changes between loopback IPs never count as an IP change (rs:621-625 `!ip.is_loopback()`),
/// which is why the IP-change re-request cannot be exercised on loopback.
#[tokio::test]
async fn register_peer_requests_pk_until_the_peer_registered_its_key() {
    // Arrange
    let server = TestServer::start().await;
    let mut peer = server.udp().await;

    // Act + Assert
    assert!(peer.register_peer(ID).await.request_pk, "unknown id");
    assert!(
        peer.register_peer(ID).await.request_pk,
        "RegisterPeer alone stores nothing"
    );
    assert_eq!(peer.register_pk(ID, &PeerKeys::random()).await, OK);
    assert!(!peer.register_peer(ID).await.request_pk, "pk is known");
    let mut moved = server.udp().await;
    assert!(
        !moved.register_peer(ID).await.request_pk,
        "loopback IP change is ignored"
    );
    server.stop().await;
}

/// rs:376: RegisterPeer with an empty id or one longer than `MAX_PEER_ID_LEN` gets no answer.
#[tokio::test]
async fn register_peer_with_empty_or_overlong_id_is_ignored() {
    // Arrange
    let server = TestServer::start().await;
    let mut peer = server.udp().await;

    // Act + Assert
    for id in [String::new(), "x".repeat(MAX_ID_LEN + 1)] {
        peer.send(&register_peer(&id)).await;
        peer.assert_silent().await;
    }
    assert!(peer.register_peer(&"x".repeat(MAX_ID_LEN)).await.request_pk);
    server.stop().await;
}

/// rs:391-393: RegisterPk without uuid or without pk is dropped without an answer.
#[tokio::test]
async fn register_pk_without_uuid_or_pk_is_ignored() {
    // Arrange
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();
    let no_uuid = PeerKeys {
        uuid: Default::default(),
        ..keys.clone()
    };
    let no_pk = PeerKeys {
        pk: Default::default(),
        ..keys
    };

    // Act + Assert
    for keys in [no_uuid, no_pk] {
        peer.send(&register_pk(ID, &keys)).await;
        peer.assert_silent().await;
    }
    server.stop().await;
}

/// rs:396-397: ids shorter than 6 or longer than 253 bytes get UUID_MISMATCH (upstream never
/// uses the protocol's INVALID_ID_FORMAT); the bounds themselves are accepted.
#[tokio::test]
async fn register_pk_with_invalid_id_length_gets_uuid_mismatch() {
    // Arrange
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();
    let cases = [
        ("1".repeat(MIN_PK_ID_LEN - 1), UUID_MISMATCH),
        ("2".repeat(MAX_ID_LEN + 1), UUID_MISMATCH),
        ("3".repeat(MIN_PK_ID_LEN), OK),
        ("4".repeat(MAX_ID_LEN), OK),
    ];

    // Act + Assert
    for (id, expected) in cases {
        assert_eq!(
            peer.register_pk(&id, &keys).await,
            expected,
            "id len {}",
            id.len()
        );
    }
    server.stop().await;
}

/// rs:401-405, 465-473: the first RegisterPk of an id stores uuid / pk / address and answers
/// OK; repeating it unchanged answers OK again.
#[tokio::test]
async fn register_pk_new_id_then_repeat_is_ok() {
    // Arrange
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();

    // Act + Assert
    assert_eq!(peer.register_pk(ID, &keys).await, OK);
    assert_eq!(peer.register_pk(ID, &keys).await, OK);
    server.stop().await;
}

/// rs:407, 420-428: another uuid for a registered id gets UUID_MISMATCH (anti id-hijack), and
/// the owner keeps the id: punch holes still reach the owner's address.
#[tokio::test]
async fn register_pk_with_other_uuid_gets_uuid_mismatch_and_owner_keeps_id() {
    // Arrange
    let server = TestServer::start().await;
    let (mut owner, _) = server.online_peer(ID).await;
    let mut thief = server.udp().await;

    // Act
    let res = thief.register_pk(ID, &PeerKeys::random()).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;

    // Assert
    assert_eq!(res, UUID_MISMATCH);
    expect_msg!(owner.recv().await, PunchHole);
    thief.assert_silent().await;
    server.stop().await;
}

/// rs:408-419: the right uuid from a new IP with a new pk gets UUID_MISMATCH.
#[tokio::test]
async fn register_pk_same_uuid_new_ip_and_new_pk_gets_uuid_mismatch() {
    // Arrange
    let server = TestServer::start().await;
    let (_owner, keys) = server.online_peer(ID).await;
    let mut moved = server.udp().await;

    // Act
    let res = moved.register_pk(ID, &keys.with_new_pk()).await;

    // Assert
    assert_eq!(res, UUID_MISMATCH);
    server.stop().await;
}

/// rs:408 needs both changes: a new pk from the same IP, then the same (new) pk from a new
/// IP, are accepted, and the latter moves the peer (rs:466 `update_pk` stores the address).
#[tokio::test]
async fn register_pk_same_uuid_changing_only_pk_or_only_ip_is_ok() {
    // Arrange: three RegisterPk for one id, the most rs:437-446 allows within 6 s.
    let server = TestServer::start().await;
    let (mut owner, keys) = server.online_peer(ID).await;
    let rotated = keys.with_new_pk();
    let mut moved = server.udp().await;

    // Act + Assert
    assert_eq!(owner.register_pk(ID, &rotated).await, OK, "new pk, same IP");
    assert_eq!(moved.register_pk(ID, &rotated).await, OK, "same pk, new IP");
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    expect_msg!(moved.recv().await, PunchHole);
    owner.assert_silent().await;
    server.stop().await;
}

/// rs:596-604: RegisterPk over TCP answers NOT_SUPPORT, and RegisterPeer over TCP is ignored
/// (the connection is closed without an answer). M1-08 implements both over TCP.
#[tokio::test]
async fn register_over_tcp_is_not_supported_upstream_behavior_m1_08_changes_this() {
    // Arrange
    let server = TestServer::start().await;
    let mut pk_conn = server.tcp().await;
    let mut peer_conn = server.tcp().await;

    // Act
    pk_conn.send(&register_pk(ID, &PeerKeys::random())).await;
    peer_conn.send(&register_peer(ID)).await;

    // Assert
    let res = expect_msg!(pk_conn.recv().await, RegisterPkResponse);
    assert_eq!(res.result.enum_value(), Ok(NOT_SUPPORT));
    peer_conn.assert_silent().await;
    server.stop().await;
}
