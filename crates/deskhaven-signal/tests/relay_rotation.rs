//! Upstream-behavior baseline (M1-04): relay round-robin.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit). Upstream's
//! rotation cursor `ROTATION_RELAY_SERVER` was process-wide, hence this binary of its own;
//! since M1-06 it is per server (`AllowAll`), which keeps the order deterministic too. Relays
//! are TEST-NET-1 (RFC 5737), so the health check never replaces the list.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::tokio;
use std::collections::HashSet;
use support::{TestServer, next_relay, wait_for_relay};

const RELAYS: [&str; 3] = ["192.0.2.1:21117", "192.0.2.2:21117", "192.0.2.3:21117"];

/// rs:965-966: each PunchHole advances the cursor by one, so N consecutive punch holes over
/// N relays name every relay once, and consecutive picks always differ.
#[tokio::test]
async fn consecutive_punch_holes_rotate_through_every_relay() {
    // Arrange
    let server = TestServer::start_with_relays(&RELAYS).await;
    let (mut target, _) = server.online_peer("rotation-target").await;
    let req = server.punch_req("rotation-target");
    wait_for_relay(&server, &mut target, &req, |r| !r.is_empty()).await;

    // Act
    let mut picks = Vec::new();
    for _ in 0..RELAYS.len() * 2 {
        picks.push(next_relay(&server, &mut target, &req).await);
    }

    // Assert
    for window in picks.chunks(RELAYS.len()) {
        let seen: HashSet<&str> = window.iter().map(String::as_str).collect();
        assert_eq!(seen, HashSet::from(RELAYS), "{picks:?}");
    }
    for pair in picks.windows(2) {
        assert_ne!(pair[0], pair[1], "{picks:?}");
    }
    server.stop().await;
}
