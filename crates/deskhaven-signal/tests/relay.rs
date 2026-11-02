//! Upstream-behavior baseline (M1-04): relay requests and relay selection.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit). Relay
//! addresses are TEST-NET-1 (RFC 5737), so the health check never finds them healthy and never
//! replaces the list. This binary asserts only membership; the rotation order is pinned in
//! `relay_rotation.rs`.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{
    Authz, ConnectReq, ControlPermissions, ControlPlane, Grant, HttpProxyRequest,
    HttpProxyResponse, RegisterInfo, RegisterVerdict, SignalConfig,
};
use hbb_common::{
    rendezvous_proto::{ControlledContext, IdPk, NatType, RequestRelay},
    tokio,
};
use std::{
    net::{IpAddr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use support::{
    CLIENT_VERSION, Conn, MAX_RELAY_SERVER_LEN, MAX_RELAY_UUID_LEN, Rendezvous, TestServer,
    decode_addr, expect_msg, max_size_request_relay, next_relay, open_id_pk, punch_hole_sent,
    relay_response, relay_via, request_relay, rv, wait_for_relay,
};

const TARGET: &str = "relay-target-1";
const RELAY_1: &str = "192.0.2.1:21117";
const RELAY_2: &str = "192.0.2.2:21117";
const RELAY_3: &str = "192.0.2.3:21117";
const RELAY_4: &str = "192.0.2.4:21117";
/// Punch holes sampled after a relay list took effect.
const SAMPLES: usize = 6;

fn relays(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// Changed in M1-08. Upstream (rs:541-554) passed a RequestRelay for a peer in memory on over
/// UDP unchanged except for socket_addr (the controller's address), i.e. including the
/// controller's `token`, `licence_key` and `switch_code`, in clear (07 §2.4, P0), and whatever
/// `control_permissions` / `controlled_context` the controller put in, which the target applies
/// over its own local settings (connection.rs `permission`) although only the server may set
/// them (Pro, M5). All five are now cleared before forwarding; everything else is passed on as
/// before. The controller gets no direct answer.
#[tokio::test]
async fn request_relay_is_forwarded_without_secrets_or_server_only_fields() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    let req = RequestRelay {
        switch_code: "switch-code".to_owned(),
        control_permissions: Some(ControlPermissions {
            permissions: u64::MAX,
            ..Default::default()
        })
        .into(),
        controlled_context: Some(ControlledContext {
            conn_audit_ref: "forged-audit-ref".to_owned(),
            ..Default::default()
        })
        .into(),
        ..request_relay(TARGET, &server.licence_key)
    };
    assert!(!req.token.is_empty() && !req.licence_key.is_empty());

    // Act
    controller.request_relay(req.clone()).await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);

    // Assert
    assert_eq!(decode_addr(&fwd.socket_addr), controller.local_addr());
    assert_eq!(fwd.token, "");
    assert_eq!(fwd.licence_key, "");
    assert_eq!(fwd.switch_code, "");
    assert!(fwd.control_permissions.is_none());
    assert!(fwd.controlled_context.is_none());
    let expected = RequestRelay {
        socket_addr: fwd.socket_addr.clone(),
        token: String::new(),
        licence_key: String::new(),
        switch_code: String::new(),
        control_permissions: Default::default(),
        controlled_context: Default::default(),
        ..req
    };
    assert_eq!(fwd, expected);
    controller.assert_silent().await;
    server.stop().await;
}

/// Changed in M1-08. Upstream (rs:546-553, no else branch) did not answer a RequestRelay for an
/// id not in memory at all, so the client waited for each of its 3 attempts and then failed
/// with "Timeout". It now gets RelayResponse{refuse_reason} at once, worded like the client's
/// own PunchHoleResponse ID_NOT_EXIST message (a translation key, client.rs:1838-1839, 1007).
#[tokio::test]
async fn request_relay_to_unknown_peer_is_refused_with_id_does_not_exist() {
    // Arrange
    let server = TestServer::start().await;
    let mut controller = server.tcp().await;
    let req = request_relay("unknown-peer", &server.licence_key);

    // Act
    controller.request_relay(req.clone()).await;
    let res = expect_msg!(controller.recv().await, RelayResponse);

    // Assert
    assert_eq!(res.refuse_reason, "ID does not exist");
    assert_eq!(res.uuid, req.uuid);
    assert!(res.relay_server.is_empty() && res.socket_addr.is_empty());
    server.stop().await;
}

/// M1-08 review (M2): strings a client hands the server to pass on to another client are
/// bounded (uuid: 64, relay_server: 259). A RequestRelay at the bounds is forwarded; one over
/// them is dropped (the target hears nothing) and its connection closed; a RelayResponse /
/// PunchHoleSent with an oversized relay_server never reaches the controller.
#[tokio::test]
async fn forwarded_strings_over_their_bounds_are_dropped() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let at_bounds = max_size_request_relay(TARGET, &server.licence_key);
    let long_uuid = RequestRelay {
        uuid: "u".repeat(MAX_RELAY_UUID_LEN + 1),
        ..at_bounds.clone()
    };
    let long_relay = RequestRelay {
        relay_server: "r".repeat(MAX_RELAY_SERVER_LEN + 1),
        ..at_bounds.clone()
    };

    // Act + Assert: requests.
    let mut ok = server.tcp().await;
    ok.request_relay(at_bounds.clone()).await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);
    assert_eq!(fwd.uuid, at_bounds.uuid);
    for req in [long_uuid, long_relay] {
        let mut controller = server.tcp().await;
        controller.request_relay(req).await;
        controller.assert_closed().await;
        target.assert_silent().await;
    }

    // Act + Assert: answers to the controller `ok`.
    let long = "r".repeat(MAX_RELAY_SERVER_LEN + 1);
    let mut answer = relay_response(&fwd, TARGET);
    answer.relay_server = long.clone();
    server
        .tcp()
        .await
        .send(&rv(|m| m.set_relay_response(answer)))
        .await;
    let sent = punch_hole_sent(&fwd.socket_addr, TARGET, &long);
    server
        .tcp()
        .await
        .send(&rv(|m| m.set_punch_hole_sent(sent)))
        .await;
    ok.assert_silent().await;
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
    let server = TestServer::start_with_relays(&[RELAY_1]).await;
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

/// rs:965-966: with several relays each PunchHole names one of the configured ones.
#[tokio::test]
async fn punch_hole_relay_is_a_member_of_the_configured_set() {
    // Arrange
    let set = [RELAY_1, RELAY_2, RELAY_3];
    let server = TestServer::start_with_relays(&set).await;
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
/// once applied, punch holes on new connections only name relays of the new set.
#[tokio::test]
async fn relay_set_replaced_through_handle_is_used_by_new_connections() {
    // Arrange
    let (old, new) = ([RELAY_1, RELAY_2], [RELAY_3, RELAY_4]);
    let server = TestServer::start_with_relays(&old).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let req = server.punch_req(TARGET);
    wait_for_relay(&server, &mut target, &req, |r| old.contains(&r)).await;

    // Act
    server.handle.set_relay_servers(relays(&new)).await.unwrap();

    // Assert
    wait_for_relay(&server, &mut target, &req, |r| new.contains(&r)).await;
    for _ in 0..SAMPLES {
        let relay = next_relay(&server, &mut target, &req).await;
        assert!(new.contains(&relay.as_str()), "{relay:?} not in {new:?}");
    }
    server.stop().await;
}

/// Changed in M1-06. Upstream (rs:1174-1182) cloned the relay list into each connection task
/// at accept, so a controller connection accepted before a relay update kept handing out the
/// old set. The list is now shared by every connection: once an update took effect, the
/// already open connection's next punch holes name only relays of the new set.
#[tokio::test]
async fn early_connection_uses_relay_set_replaced_later() {
    // Arrange
    let (old, new) = ([RELAY_1, RELAY_2], [RELAY_3, RELAY_4]);
    let server = TestServer::start_with_relays(&old).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let req = server.punch_req(TARGET);
    wait_for_relay(&server, &mut target, &req, |r| old.contains(&r)).await;
    let mut early = server.tcp().await;
    let before = relay_via(&mut early, &mut target, &req).await;

    // Act
    server.handle.set_relay_servers(relays(&new)).await.unwrap();
    wait_for_relay(&server, &mut target, &req, |r| new.contains(&r)).await;

    // Assert
    assert!(old.contains(&before.as_str()), "{before:?}");
    for _ in 0..SAMPLES {
        let after = relay_via(&mut early, &mut target, &req).await;
        assert!(new.contains(&after.as_str()), "early connection: {after:?}");
    }
    server.stop().await;
}

/// M1-06: relay choices go through the injected `ControlPlane::pick_relay`, which sees the
/// controller's and the target's addresses and the current relay list; its answer is what the
/// target is told (`None`: no relay).
#[tokio::test]
async fn control_plane_picks_the_relay_handed_out() {
    // Arrange
    let control = Arc::new(FixedPick::default());
    let server = TestServer::start_with_control(
        |cfg| SignalConfig {
            relay_servers: relays(&[RELAY_1, RELAY_2]),
            ..cfg
        },
        control.clone(),
    )
    .await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let req = server.punch_req(TARGET);
    let resolved = || {
        control
            .last
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|c| c.2.len() == 2)
    };
    wait_for_relay(&server, &mut target, &req, |_| resolved()).await;
    let mut controller = server.tcp().await;

    // Act
    let picked = relay_via(&mut controller, &mut target, &req).await;
    control.answer.store(false, Ordering::SeqCst);
    let none = relay_via(&mut controller, &mut target, &req).await;

    // Assert
    assert_eq!(picked, PICKED);
    assert_eq!(none, "");
    let (a, b, list) = control.last.lock().unwrap().clone().unwrap();
    assert_eq!(a, controller.local_addr().ip());
    assert_eq!(b, target.local_addr().ip());
    assert_eq!(list, relays(&[RELAY_1, RELAY_2]));
    server.stop().await;
}

const PICKED: &str = "picked.example:21117";

/// Answers [`PICKED`] (or `None` once `answer` is cleared) and records the last call.
struct FixedPick {
    answer: AtomicBool,
    last: Mutex<Option<(IpAddr, IpAddr, Vec<String>)>>,
}

impl Default for FixedPick {
    fn default() -> Self {
        Self {
            answer: AtomicBool::new(true),
            last: Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl ControlPlane for FixedPick {
    fn register_pk(&self, _: &RegisterInfo<'_>) -> RegisterVerdict {
        RegisterVerdict::Ok
    }
    async fn authorize(&self, _: &ConnectReq<'_>) -> Authz {
        Authz::Allow(Grant::default())
    }
    async fn http_proxy(&self, _: SocketAddr, _: HttpProxyRequest) -> HttpProxyResponse {
        HttpProxyResponse::default()
    }
    async fn health_check(&self, _: &str, _: SocketAddr) -> Option<i32> {
        None
    }
    fn pick_relay(&self, a: IpAddr, b: IpAddr, relays: &[String]) -> Option<String> {
        *self.last.lock().unwrap() = Some((a, b, relays.to_vec()));
        let answer = self.answer.load(Ordering::SeqCst);
        answer.then(|| PICKED.to_owned())
    }
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
