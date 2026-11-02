//! Upstream-behavior baseline (M1-04): RegisterPk rate limits (TOO_FREQUENT).
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit). Its own
//! test binary because the per-IP limit lives in the process-wide `IP_BLOCKER`; every client
//! still uses its own loopback source IP, so even these tests cannot see each other's counts.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    rendezvous_proto::register_pk_response::Result::{OK, TOO_FREQUENT},
    tokio::{
        self,
        time::{Duration, sleep},
    },
};
use support::{PeerKeys, TestServer};

/// rs:437-446: accepted RegisterPk per peer within the window.
const PER_PEER_LIMIT: usize = 3;
/// rs:438 `elapsed().as_secs() > 6`: the window resets once 7 whole seconds have passed
/// since the last accepted RegisterPk (plus a margin).
const PER_PEER_WINDOW_RESET: Duration = Duration::from_millis(7_250);
/// rs:926-934 + 948: the first RegisterPk of an IP inserts a counter at 0, each later one
/// increments it and is refused once it exceeds 30, so 32 are accepted per 60 s.
const PER_IP_LIMIT: usize = 32;

/// rs:437-446: per peer at most 3 RegisterPk within 6 s, then TOO_FREQUENT (which does not
/// restart the window) until more than 6 whole seconds passed since the last accepted one.
#[tokio::test]
async fn fourth_register_pk_within_6s_is_too_frequent_until_window_passes() {
    // Arrange
    let server = TestServer::start().await;
    let mut peer = server.udp().await;
    let keys = PeerKeys::random();
    for i in 0..PER_PEER_LIMIT {
        assert_eq!(peer.register_pk("123456789", &keys).await, OK, "#{i}");
    }

    // Act + Assert
    assert_eq!(peer.register_pk("123456789", &keys).await, TOO_FREQUENT);
    sleep(PER_PEER_WINDOW_RESET).await;
    assert_eq!(peer.register_pk("123456789", &keys).await, OK);
    server.stop().await;
}

/// rs:398-399, 923-951 `check_ip_blocker`: per source IP, 32 RegisterPk (distinct ids, so the
/// per-peer limit never applies) are accepted within 60 s and the 33rd is TOO_FREQUENT; another
/// IP is unaffected.
#[tokio::test]
async fn thirty_third_register_pk_from_one_ip_within_60s_is_too_frequent() {
    // Arrange
    let server = TestServer::start().await;
    let mut busy = server.udp().await;
    let keys = PeerKeys::random();
    for i in 0..PER_IP_LIMIT {
        let id = format!("peer-{i:04}");
        assert_eq!(busy.register_pk(&id, &keys).await, OK, "#{i}");
    }

    // Act
    let refused = busy.register_pk("peer-over-limit", &keys).await;
    let other_ip = server
        .udp()
        .await
        .register_pk("peer-over-limit", &keys)
        .await;

    // Assert
    assert_eq!(refused, TOO_FREQUENT);
    assert_eq!(other_ip, OK);
    server.stop().await;
}
