//! Relay addresses are TEST-NET-1, so the health check never finds them healthy and never
//! replaces the list; the rotation order is pinned in `relay_rotation.rs`.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::ControlPermissions;
use hbb_common::{
    rendezvous_proto::{ControlledContext, IdPk, RelayResponse, RequestRelay},
    tokio,
};
use support::{
    CLIENT_VERSION, Conn, MAX_RELAY_SERVER_LEN, MAX_RELAY_UUID_LEN, Rendezvous, TestServer,
    decode_addr, expect_msg, max_size_request_relay, open_id_pk, punch_hole_sent, relay_response,
    relay_via, request_relay, rv, wait_for_relay,
};

const TARGET: &str = "relay-target-1";
const RELAY_1: &str = "192.0.2.1:21117";
const RELAY_2: &str = "192.0.2.2:21117";
const RELAY_3: &str = "192.0.2.3:21117";
const RELAY_4: &str = "192.0.2.4:21117";
const SAMPLES: usize = 6;

fn relays(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// The controller's `token`, `licence_key`, `switch_code`, `control_permissions` and
/// `controlled_context` must not reach the target: only the server may set the last two.
#[tokio::test]
async fn request_relay_is_forwarded_without_secrets_or_server_only_fields() {
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

    controller.request_relay(req.clone()).await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);

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

/// Upstream left a RequestRelay for an unknown id unanswered, so the client timed out; the
/// refusal is worded like the client's own ID_NOT_EXIST message.
#[tokio::test]
async fn request_relay_to_unknown_peer_is_refused_with_id_does_not_exist() {
    let server = TestServer::start().await;
    let mut controller = server.tcp().await;
    let req = request_relay("unknown-peer", &server.licence_key);

    controller.request_relay(req.clone()).await;
    let res = expect_msg!(controller.recv().await, RelayResponse);

    assert_eq!(res.refuse_reason, "ID does not exist");
    assert_eq!(res.uuid, req.uuid);
    assert!(res.relay_server.is_empty() && res.socket_addr.is_empty());
    server.stop().await;
}

#[tokio::test]
async fn forwarded_strings_over_their_bounds_are_dropped() {
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

/// Relay uuids follow the client's rule (`[A-Za-z0-9_-]{1,128}`), so older clients that pass
/// the uuid on unchecked never get anything else.
#[tokio::test]
async fn request_relay_with_a_uuid_outside_the_client_rule_is_dropped() {
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let base = request_relay(TARGET, &server.licence_key);
    let bad = [
        "",
        "relay uuid",
        "relay/uuid",
        "relay.uuid",
        "relay-uuid-\u{e9}",
        &"u".repeat(MAX_RELAY_UUID_LEN + 1),
    ];

    for uuid in ["a", "e4c181d0-41c7-4ae6-8ef0-36581b6b2b12", "A_z-0"] {
        let mut controller = server.tcp().await;
        let req = RequestRelay {
            uuid: uuid.to_owned(),
            ..base.clone()
        };
        controller.request_relay(req).await;

        let fwd = expect_msg!(target.recv().await, RequestRelay);
        assert_eq!(fwd.uuid, uuid);
    }
    for uuid in bad {
        let mut controller = server.tcp().await;
        let req = RequestRelay {
            uuid: uuid.to_owned(),
            ..base.clone()
        };
        controller.request_relay(req).await;

        controller.assert_closed().await;
        target.assert_silent().await;
    }
    server.stop().await;
}

/// A RelayResponse without a uuid is how clients answer a RequestRelay, so it still reaches the
/// controller.
#[tokio::test]
async fn relay_response_with_a_malformed_uuid_is_dropped_and_one_without_a_uuid_passes() {
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    controller
        .request_relay(request_relay(TARGET, &server.licence_key))
        .await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);
    let answer = relay_response(&fwd, TARGET);
    let over_cap = "u".repeat(MAX_RELAY_UUID_LEN + 1);

    for uuid in [
        "relay uuid",
        "relay/uuid",
        "relay.uuid",
        "\u{e9}",
        &over_cap,
    ] {
        let mut target_tcp = server.tcp().await;
        let bad = RelayResponse {
            uuid: uuid.to_owned(),
            ..answer.clone()
        };
        target_tcp.send(&rv(|m| m.set_relay_response(bad))).await;

        target_tcp.assert_closed().await;
        controller.assert_silent().await;
    }

    let unset = RelayResponse {
        uuid: String::new(),
        ..answer
    };
    server
        .tcp()
        .await
        .send(&rv(|m| m.set_relay_response(unset)))
        .await;
    let res = expect_msg!(controller.recv().await, RelayResponse);

    assert_eq!(res.uuid, "");
    assert_eq!(res.relay_server, fwd.relay_server);
    server.stop().await;
}

#[tokio::test]
async fn relay_response_from_target_reaches_controller_with_signed_pk() {
    let server = TestServer::start().await;
    let (mut target, keys) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    controller
        .request_relay(request_relay(TARGET, &server.licence_key))
        .await;
    let fwd = expect_msg!(target.recv().await, RequestRelay);
    let mut target_tcp = server.tcp().await;

    let answer = relay_response(&fwd, TARGET);
    target_tcp.send(&rv(|m| m.set_relay_response(answer))).await;
    let res = expect_msg!(controller.recv().await, RelayResponse);

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

#[tokio::test]
async fn early_connection_uses_relay_set_replaced_later() {
    let (old, new) = ([RELAY_1, RELAY_2], [RELAY_3, RELAY_4]);
    let server = TestServer::start_with_relays(&old).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let req = server.punch_req(TARGET);
    wait_for_relay(&server, &mut target, &req, |r| old.contains(&r)).await;
    let mut early = server.tcp().await;
    let before = relay_via(&mut early, &mut target, &req).await;

    server.handle.set_relay_servers(relays(&new)).await.unwrap();
    wait_for_relay(&server, &mut target, &req, |r| new.contains(&r)).await;

    assert!(old.contains(&before.as_str()), "{before:?}");
    for _ in 0..SAMPLES {
        let after = relay_via(&mut early, &mut target, &req).await;
        assert!(new.contains(&after.as_str()), "early connection: {after:?}");
    }
    server.stop().await;
}
