//! Baseline of upstream hbbs registration behavior; tests that change it say so in their name.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{MemoryPeerStore, PeerStore};
use hbb_common::{
    rendezvous_proto::{
        punch_hole_response::Failure::ID_NOT_EXIST,
        register_pk_response::Result::{OK, UUID_MISMATCH},
    },
    tokio,
};
use std::sync::Arc;
use support::{
    Conn, PeerKeys, Rendezvous, TestServer, decode_addr, expect_msg, register_peer, register_pk,
};

const ID: &str = "123456789";
const MAX_ID_LEN: usize = 253;
/// RegisterPk ids shorter than this are refused.
const MIN_PK_ID_LEN: usize = 6;
/// Valid lengths, but not peer ids: control and invisible characters, space, and a non-ASCII id
/// the client's custom id rule refuses.
const NOT_PEER_IDS: [&str; 8] = [
    "10\u{0}aaa",
    "abc\ndef",
    "abc\rdef",
    "abc\u{200b}def",
    "abc\u{200d}def",
    "abc\u{202e}def",
    "abc def",
    "\u{e9}bcdef",
];
/// A default numeric id, `-` / `_`, RustDesk's longest custom id, and custom ids the client
/// accepts with Unicode `\w`.
const PEER_IDS: [&str; 6] = [
    "123456789",
    "abc_def-12",
    "Custom-id_123456",
    "a\u{e9}12345",
    "a\u{5f20}\u{4e09}1234",
    "a\u{203f}bcdef",
];

/// Address changes between loopback IPs never count as an IP change, so the IP-change
/// re-request cannot be exercised on loopback.
#[tokio::test]
async fn register_peer_requests_pk_until_the_peer_registered_its_key() {
    let server = TestServer::start().await;
    let mut peer = server.udp().await;

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

#[tokio::test]
async fn register_peer_with_empty_or_overlong_id_is_ignored() {
    let server = TestServer::start().await;
    let mut peer = server.udp().await;

    for id in [String::new(), "x".repeat(MAX_ID_LEN + 1)] {
        peer.send(&register_peer(&id)).await;
        peer.assert_silent().await;
    }
    assert!(peer.register_peer(&"x".repeat(MAX_ID_LEN)).await.request_pk);
    server.stop().await;
}

#[tokio::test]
async fn register_pk_without_uuid_or_pk_is_ignored() {
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

    for keys in [no_uuid, no_pk] {
        peer.send(&register_pk(ID, &keys)).await;
        peer.assert_silent().await;
    }
    server.stop().await;
}

#[tokio::test]
async fn register_pk_with_malformed_pk_or_overlong_uuid_is_ignored() {
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();
    let with = |uuid: Vec<u8>, pk: Vec<u8>| PeerKeys {
        uuid: uuid.into(),
        pk: pk.into(),
    };
    let malformed = [
        with(keys.uuid.to_vec(), vec![7; 31]),
        with(keys.uuid.to_vec(), vec![7; 33]),
        with(vec![1; 129], keys.pk.to_vec()),
    ];

    for keys in malformed {
        peer.send(&register_pk(ID, &keys)).await;
        peer.assert_silent().await;
    }
    assert!(
        peer.register_peer(ID).await.request_pk,
        "nothing registered"
    );
    let longest_uuid = with(vec![1; 128], keys.pk.to_vec());
    assert_eq!(peer.register_pk(ID, &longest_uuid).await, OK);
    server.stop().await;
}

/// Upstream answers bad lengths with UUID_MISMATCH, never the protocol's INVALID_ID_FORMAT.
#[tokio::test]
async fn register_pk_with_invalid_id_length_gets_uuid_mismatch() {
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();
    let cases = [
        ("1".repeat(MIN_PK_ID_LEN - 1), UUID_MISMATCH),
        ("2".repeat(MAX_ID_LEN + 1), UUID_MISMATCH),
        ("3".repeat(MIN_PK_ID_LEN), OK),
        ("4".repeat(MAX_ID_LEN), OK),
    ];

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

#[tokio::test]
async fn register_pk_with_a_malformed_id_gets_uuid_mismatch() {
    let store = Arc::new(MemoryPeerStore::new());
    let server = TestServer::start_with_store(|cfg| cfg, store.clone()).await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();

    for id in NOT_PEER_IDS {
        assert_eq!(peer.register_pk(id, &keys).await, UUID_MISMATCH, "{id:?}");
    }
    for id in PEER_IDS {
        assert_eq!(peer.register_pk(id, &keys).await, OK, "{id:?}");
    }
    for id in NOT_PEER_IDS {
        let mut controller = server.tcp().await;
        controller.punch_hole_request(server.punch_req(id)).await;
        let res = expect_msg!(controller.recv().await, PunchHoleResponse);
        assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST), "{id:?}");
    }
    server.stop().await;
    let mut stored: Vec<String> = store
        .load_page(None, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|rec| rec.id)
        .collect();
    stored.sort();
    let mut expected = PEER_IDS.map(str::to_owned).to_vec();
    expected.sort();
    assert_eq!(stored, expected);
}

/// RegisterPeer keeps the length-only check: the server's own self-test probes with an id that
/// is not a peer id.
#[tokio::test]
async fn register_peer_with_a_malformed_id_stays_unknown() {
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();

    for id in NOT_PEER_IDS {
        assert_eq!(peer.register_pk(id, &keys).await, UUID_MISMATCH, "{id:?}");
        assert!(peer.register_peer(id).await.request_pk, "{id:?}");
    }
    server.stop().await;
}

#[tokio::test]
async fn register_pk_stores_peer_and_repeat_is_ok() {
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();

    let first = peer.register_pk(ID, &keys).await;
    let again = peer.register_pk(ID, &keys).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;

    assert_eq!((first, again), (OK, OK));
    let ph = expect_msg!(peer.recv().await, PunchHole);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    server.stop().await;
}

#[tokio::test]
async fn restart_keeps_registrations_with_persistent_store() {
    let server = TestServer::start().await;
    let (mut owner, keys) = server.online_peer(ID).await;

    let server = server.restart().await;
    let mut thief = server.udp().await;
    let other_uuid = thief.register_pk(ID, &PeerKeys::random()).await;
    let mut moved = server.udp().await;
    let new_ip_and_pk = moved.register_pk(ID, &keys.with_new_pk()).await;
    let asked_for_pk = owner.register_peer(ID).await.request_pk;
    let owner_again = owner.register_pk(ID, &keys).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;

    assert_eq!(other_uuid, UUID_MISMATCH, "another uuid");
    assert_eq!(new_ip_and_pk, UUID_MISMATCH, "same uuid, new IP and new pk");
    assert!(!asked_for_pk, "pk known from the store");
    assert_eq!(owner_again, OK, "owner, unchanged keys");
    let ph = expect_msg!(owner.recv().await, PunchHole);
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    thief.assert_silent().await;
    moved.assert_silent().await;
    server.stop().await;
}

#[tokio::test]
async fn register_pk_with_other_uuid_gets_uuid_mismatch_and_owner_keeps_id() {
    let server = TestServer::start().await;
    let (mut owner, _) = server.online_peer(ID).await;
    let mut thief = server.udp().await;

    let res = thief.register_pk(ID, &PeerKeys::random()).await;
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;

    assert_eq!(res, UUID_MISMATCH);
    expect_msg!(owner.recv().await, PunchHole);
    thief.assert_silent().await;
    server.stop().await;
}

#[tokio::test]
async fn register_pk_same_uuid_new_ip_and_new_pk_gets_uuid_mismatch() {
    let server = TestServer::start().await;
    let (_owner, keys) = server.online_peer(ID).await;
    let mut moved = server.udp().await;

    let res = moved.register_pk(ID, &keys.with_new_pk()).await;

    assert_eq!(res, UUID_MISMATCH);
    server.stop().await;
}

#[tokio::test]
async fn register_pk_same_uuid_changing_only_pk_or_only_ip_is_ok() {
    // Three RegisterPk for one id, the most allowed within 6 s.
    let server = TestServer::start().await;
    let (mut owner, keys) = server.online_peer(ID).await;
    let rotated = keys.with_new_pk();
    let mut moved = server.udp().await;

    assert_eq!(owner.register_pk(ID, &rotated).await, OK, "new pk, same IP");
    assert_eq!(moved.register_pk(ID, &rotated).await, OK, "same pk, new IP");
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    expect_msg!(moved.recv().await, PunchHole);
    owner.assert_silent().await;
    server.stop().await;
}
