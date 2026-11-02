//! Upstream-behavior baseline (M1-04): relay requests and relay selection.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit). Relay
//! addresses are TEST-NET-1 (RFC 5737), so the health check never finds them healthy and never
//! replaces the list. Relay selection uses the process-wide `ROTATION_RELAY_SERVER` cursor,
//! so only membership in the configured set is asserted, never the order.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::SignalConfig;
use hbb_common::{
    rendezvous_proto::{IdPk, NatType, PunchHoleRequest, RequestRelay},
    tokio::{
        self,
        time::{Duration, Instant, sleep},
    },
};
use support::{
    CLIENT_VERSION, Conn, Rendezvous, STEP, TcpClient, TestServer, UdpClient, decode_addr,
    expect_msg, open_id_pk, relay_response, request_relay, rv,
};

const TARGET: &str = "relay-target-1";
const RELAY_1: &str = "192.0.2.1:21117";
const RELAY_2: &str = "192.0.2.2:21117";
const RELAY_3: &str = "192.0.2.3:21117";
const RELAY_4: &str = "192.0.2.4:21117";
/// Punch holes sampled after a relay list took effect.
const SAMPLES: usize = 6;
const POLL: Duration = Duration::from_millis(50);

fn relays(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

async fn start_with_relays(list: &[&str]) -> TestServer {
    TestServer::start_with(|cfg| SignalConfig {
        relay_servers: relays(list),
        ..cfg
    })
    .await
}

/// The relay_server of the PunchHole forwarded for a request sent on `controller`.
async fn relay_via(
    controller: &mut TcpClient,
    target: &mut UdpClient,
    req: &PunchHoleRequest,
) -> String {
    controller.punch_hole_request(req.clone()).await;
    expect_msg!(target.recv().await, PunchHole).relay_server
}

/// [`relay_via`] a new controller connection. A connection keeps the relay list that was
/// current when it was accepted (rs:1174-1182 clones the server state into each connection
/// task), so only new connections see a new list.
async fn next_relay(server: &TestServer, target: &mut UdpClient, req: &PunchHoleRequest) -> String {
    relay_via(&mut server.tcp().await, target, req).await
}

/// Repeats [`next_relay`] until the relay satisfies `accept`: relay lists apply asynchronously
/// (initial config at the first timer tick, handle updates through the server loop).
async fn wait_for_relay(
    server: &TestServer,
    target: &mut UdpClient,
    req: &PunchHoleRequest,
    accept: impl Fn(&str) -> bool,
) -> String {
    let deadline = Instant::now() + STEP;
    loop {
        let relay = next_relay(server, target, req).await;
        if accept(&relay) {
            return relay;
        }
        assert!(Instant::now() < deadline, "relay {relay:?} never accepted");
        sleep(POLL).await;
    }
}

/// rs:541-554: RequestRelay for a peer in memory is passed on over UDP unchanged except for
/// socket_addr (the controller's address), i.e. including the controller's `token` and
/// `licence_key`, in clear (07 §2.4, P0). The controller gets no direct answer. M1-08 clears
/// both fields before forwarding.
#[tokio::test]
async fn request_relay_forwards_token_and_licence_key_upstream_behavior_m1_08_changes_this() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    let req = request_relay(TARGET, &server.licence_key);

    // Act
    controller.request_relay(req.clone()).await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);

    // Assert
    assert_eq!(decode_addr(&fwd.socket_addr), controller.local_addr());
    assert_eq!(fwd.token, req.token);
    assert_eq!(fwd.licence_key, server.licence_key);
    let expected = RequestRelay {
        socket_addr: fwd.socket_addr.clone(),
        ..req
    };
    assert_eq!(fwd, expected);
    controller.assert_silent().await;
    server.stop().await;
}

/// rs:546-553 has no else branch: RequestRelay for an id not in memory gets no answer at all
/// (clients wait and retry). M1-08 answers RelayResponse{refuse_reason}.
#[tokio::test]
async fn request_relay_to_unknown_peer_gets_no_answer_upstream_behavior_m1_08_changes_this() {
    // Arrange
    let server = TestServer::start().await;
    let mut controller = server.tcp().await;

    // Act
    controller
        .request_relay(request_relay("unknown-peer", &server.licence_key))
        .await;

    // Assert
    controller.assert_silent().await;
    server.stop().await;
}

/// rs:555-574, 1245 `get_pk`: the target's RelayResponse (over its own TCP connection) reaches
/// the controller with socket_addr cleared and the id replaced by the target's pk signed by
/// the server key; uuid, relay_server and version are unchanged.
#[tokio::test]
async fn relay_response_from_target_reaches_controller_with_signed_pk() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, keys) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    controller
        .request_relay(request_relay(TARGET, &server.licence_key))
        .await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);
    let mut target_tcp = server.tcp().await;

    // Act
    let answer = relay_response(&fwd, TARGET);
    target_tcp.send(&rv(|m| m.set_relay_response(answer))).await;
    let res = expect_msg!(controller.recv().await, RelayResponse);

    // Assert
    assert!(res.socket_addr.is_empty());
    let expected_pk = IdPk {
        id: TARGET.to_owned(),
        pk: keys.pk.clone(),
        ..Default::default()
    };
    assert_eq!(open_id_pk(res.pk(), &server.public_key), expected_pk);
    assert_eq!(res.uuid, fwd.uuid);
    assert_eq!(res.relay_server, fwd.relay_server);
    assert_eq!(res.version, CLIENT_VERSION);
    server.stop().await;
}

/// rs:962-963: a single configured relay is handed out in every PunchHole.
#[tokio::test]
async fn single_configured_relay_is_handed_out_in_every_punch_hole() {
    // Arrange
    let server = start_with_relays(&[RELAY_1]).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let req = server.punch_req(TARGET);

    // Act + Assert
    let first = wait_for_relay(&server, &mut target, &req, |r| !r.is_empty()).await;
    assert_eq!(first, RELAY_1);
    for _ in 0..SAMPLES {
        assert_eq!(next_relay(&server, &mut target, &req).await, RELAY_1);
    }
    server.stop().await;
}

/// rs:965-966: with several relays each PunchHole names one of the configured ones
/// (round-robin over the process-wide cursor; order not asserted).
#[tokio::test]
async fn punch_hole_relay_is_a_member_of_the_configured_set() {
    // Arrange
    let set = [RELAY_1, RELAY_2, RELAY_3];
    let server = start_with_relays(&set).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let req = server.punch_req(TARGET);

    // Act + Assert
    wait_for_relay(&server, &mut target, &req, |r| !r.is_empty()).await;
    for _ in 0..SAMPLES {
        let relay = next_relay(&server, &mut target, &req).await;
        assert!(set.contains(&relay.as_str()), "{relay:?} not in {set:?}");
    }
    server.stop().await;
}

/// `SignalHandle::set_relay_servers` (replaces the console `rs` command, CHANGES.md M1-03):
/// once applied, punch holes on new connections only name relays of the new set, while a
/// connection accepted before keeps the old set (rs:1174-1182: each connection task works on
/// a clone of the server state, including `relay_servers`).
#[tokio::test]
async fn relay_set_replaced_through_handle_applies_to_new_connections_only() {
    // Arrange
    let (old, new) = ([RELAY_1, RELAY_2], [RELAY_3, RELAY_4]);
    let server = start_with_relays(&old).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let req = server.punch_req(TARGET);
    wait_for_relay(&server, &mut target, &req, |r| old.contains(&r)).await;
    let mut early = server.tcp().await;
    let before = relay_via(&mut early, &mut target, &req).await;

    // Act
    server.handle.set_relay_servers(relays(&new)).await.unwrap();

    // Assert
    assert!(old.contains(&before.as_str()), "{before:?}");
    wait_for_relay(&server, &mut target, &req, |r| new.contains(&r)).await;
    for _ in 0..SAMPLES {
        let relay = next_relay(&server, &mut target, &req).await;
        assert!(new.contains(&relay.as_str()), "{relay:?} not in {new:?}");
    }
    let after = relay_via(&mut early, &mut target, &req).await;
    assert!(old.contains(&after.as_str()), "early connection: {after:?}");
    server.stop().await;
}

/// rs:771-777: ALWAYS_USE_RELAY (now `SignalHandle::set_always_use_relay`) forces nat_type
/// SYMMETRIC in the forwarded PunchHole, immediately; switched off, the request's nat_type is
/// forwarded again.
#[tokio::test]
async fn always_use_relay_forces_symmetric_nat_type() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    let req = server.punch_req(TARGET);
    assert_eq!(req.nat_type.enum_value(), Ok(NatType::ASYMMETRIC));

    for (on, expected) in [(true, NatType::SYMMETRIC), (false, NatType::ASYMMETRIC)] {
        // Act
        server.handle.set_always_use_relay(on).unwrap();
        controller.punch_hole_request(req.clone()).await;
        let ph = expect_msg!(target.recv().await, PunchHole);

        // Assert
        assert_eq!(
            ph.nat_type.enum_value(),
            Ok(expected),
            "always_use_relay={on}"
        );
    }
    server.stop().await;
}
