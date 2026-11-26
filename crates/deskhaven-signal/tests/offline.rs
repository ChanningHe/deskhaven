//! Liveness uses `std::time::Instant`, which cannot be faked, so these tests wait 30 s of real
//! time; they share this binary and run in parallel, so the suite pays the wait once.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    rendezvous_proto::punch_hole_response::Failure::OFFLINE,
    tokio::{
        self,
        time::{Duration, sleep},
    },
};
use support::{
    Conn, REG_TIMEOUT, Rendezvous, TestServer, decode_addr, expect_msg, online_bits, request_relay,
};

const TARGET: &str = "stale-target-1";
/// REG_TIMEOUT plus a margin.
const GO_STALE: Duration = REG_TIMEOUT.saturating_add(Duration::from_millis(500));
/// Checked before REG_TIMEOUT: the peer must still be online here.
const STILL_ONLINE: Duration = REG_TIMEOUT.saturating_sub(Duration::from_secs(2));

async fn is_online(server: &TestServer, id: &str) -> bool {
    let res = server.nat().await.online_request(&[id]).await;
    online_bits(&res.states, 1)[0]
}

#[tokio::test]
async fn peer_silent_for_reg_timeout_is_offline_until_it_registers_again() {
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    assert!(
        is_online(&server, TARGET).await,
        "online right after RegisterPk"
    );

    sleep(STILL_ONLINE).await;
    assert!(
        is_online(&server, TARGET).await,
        "online before REG_TIMEOUT"
    );
    let mut early = server.tcp().await;
    early.punch_hole_request(server.punch_req(TARGET)).await;
    expect_msg!(target.recv().await, PunchHole);

    sleep(GO_STALE - STILL_ONLINE).await;

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

#[tokio::test]
async fn request_relay_to_stale_peer_is_refused_as_offline() {
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    sleep(GO_STALE).await;
    assert!(!is_online(&server, TARGET).await, "stale");
    let mut controller = server.tcp().await;
    let req = request_relay(TARGET, &server.licence_key);

    controller.request_relay(req.clone()).await;
    let res = expect_msg!(controller.recv().await, RelayResponse);

    assert_eq!(res.refuse_reason, "Remote desktop is offline");
    assert_eq!(res.uuid, req.uuid);
    target.assert_silent().await;
    assert!(!target.register_peer(TARGET).await.request_pk);
    let mut again = server.tcp().await;
    again.request_relay(req).await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);
    assert_eq!(decode_addr(&fwd.socket_addr), again.local_addr());
    server.stop().await;
}
