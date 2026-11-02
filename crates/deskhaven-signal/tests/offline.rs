//! Upstream-behavior baseline (M1-04): peers going stale.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit). The server
//! measures liveness with `std::time::Instant` against REG_TIMEOUT (30 s), which cannot be
//! faked, so these tests wait 30 s of real time. They share this binary and run in parallel,
//! so the suite pays the wait once.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    rendezvous_proto::{RequestRelay, punch_hole_response::Failure::OFFLINE},
    tokio::{
        self,
        time::{Duration, sleep},
    },
};
use support::{
    Conn, REG_TIMEOUT, Rendezvous, TestServer, decode_addr, expect_msg, online_bits, request_relay,
};

const TARGET: &str = "stale-target-1";
/// rs:750 `elapsed >= REG_TIMEOUT`, plus a margin.
const GO_STALE: Duration = REG_TIMEOUT.saturating_add(Duration::from_millis(500));
/// Checked before REG_TIMEOUT: the peer must still be online here.
const STILL_ONLINE: Duration = REG_TIMEOUT.saturating_sub(Duration::from_secs(2));

async fn is_online(server: &TestServer, id: &str) -> bool {
    let res = server.nat().await.online_request(&[id]).await;
    online_bits(&res.states, 1)[0]
}

/// rs:745-757 and rs:836-842: a peer silent for less than REG_TIMEOUT is still online (punch
/// holes are forwarded, the online bit is set); silent for REG_TIMEOUT it gets
/// PunchHoleResponse{OFFLINE} and a clear online bit; a RegisterPeer heartbeat (pk known,
/// rs:627-630) brings it back.
#[tokio::test]
async fn peer_silent_for_reg_timeout_is_offline_until_it_registers_again() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    assert!(
        is_online(&server, TARGET).await,
        "online right after RegisterPk"
    );

    // Act + Assert: still online shortly before REG_TIMEOUT.
    sleep(STILL_ONLINE).await;
    assert!(
        is_online(&server, TARGET).await,
        "online before REG_TIMEOUT"
    );
    let mut early = server.tcp().await;
    early.punch_hole_request(server.punch_req(TARGET)).await;
    expect_msg!(target.recv().await, PunchHole);

    // Act
    sleep(GO_STALE - STILL_ONLINE).await;

    // Assert
    assert!(!is_online(&server, TARGET).await, "stale");
    let mut controller = server.tcp().await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);
    assert_eq!(res.failure.enum_value(), Ok(OFFLINE));
    target.assert_silent().await;
    assert!(!target.register_peer(TARGET).await.request_pk);
    assert!(is_online(&server, TARGET).await, "heartbeat refreshed");
    server.stop().await;
}

/// rs:546 looks the peer up with `get_in_memory` and never checks REG_TIMEOUT: a stale peer
/// still gets the RequestRelay. M1-08 answers offline peers with RelayResponse{refuse_reason}.
#[tokio::test]
async fn request_relay_still_forwarded_to_stale_peer_upstream_behavior_m1_08_changes_this() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    sleep(GO_STALE).await;
    assert!(!is_online(&server, TARGET).await, "stale");
    let mut controller = server.tcp().await;
    let req = request_relay(TARGET, &server.licence_key);

    // Act
    controller.request_relay(req.clone()).await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);

    // Assert
    assert_eq!(decode_addr(&fwd.socket_addr), controller.local_addr());
    let expected = RequestRelay {
        socket_addr: fwd.socket_addr.clone(),
        token: String::new(),
        licence_key: String::new(),
        ..req
    };
    assert_eq!(fwd, expected);
    server.stop().await;
}
