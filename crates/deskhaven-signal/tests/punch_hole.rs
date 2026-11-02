//! Upstream-behavior baseline (M1-04): hole punching over TCP (controller A, target B).
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit). Flow: A
//! sends PunchHoleRequest over TCP; hbbs keeps A's sink (rs:531-536) and forwards PunchHole or
//! FetchLocalAddr to B over UDP; B answers over its own TCP connection; hbbs sends the answer
//! to A's stored sink as PunchHoleResponse. Going offline (30 s) lives in `offline.rs`,
//! websocket in `websocket.rs`, relay choice in `relay.rs`.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    rendezvous_proto::{
        FetchLocalAddr, IdPk, NatType, PunchHole, PunchHoleRequest, PunchHoleResponse,
        punch_hole_response::Failure::{ID_NOT_EXIST, LICENSE_MISMATCH},
    },
    tokio,
};
use std::net::SocketAddr;
use support::{
    Conn, Rendezvous, TcpClient, TestServer, decode_addr, encode_addr, expect_msg, local_addr,
    open_id_pk, punch_hole_sent, rv,
};

const TARGET_RELAY: &str = "192.0.2.70:21117";

fn signed_pk(id: &str, pk: &[u8]) -> IdPk {
    IdPk {
        id: id.to_owned(),
        pk: pk.to_vec().into(),
        ..Default::default()
    }
}

/// rs:531-540, 889-899, 806-811: a PunchHoleRequest for an online peer reaches it over UDP
/// as PunchHole carrying the controller's address (as hbbs sees it) and the request's
/// nat_type; nothing else of the request is forwarded (no relay is configured here, so
/// relay_server is empty, rs:960-961). The controller gets no direct answer.
#[tokio::test]
async fn punch_hole_request_to_online_peer_is_forwarded_as_punch_hole_with_controller_addr() {
    const TARGET: &str = "ph-request-forwarded";
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;

    // Act
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);

    // Assert
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    let expected = PunchHole {
        socket_addr: ph.socket_addr.clone(),
        nat_type: NatType::ASYMMETRIC.into(),
        ..Default::default()
    };
    assert_eq!(ph, expected);
    controller.assert_silent().await;
    server.stop().await;
}

/// rs:575-577, 656-688, 853-859: the target's PunchHoleSent reaches the controller as
/// PunchHoleResponse with the target's TCP address, the target's pk signed by the server key
/// (rs:1245 `get_pk`, because `version` is set), and relay_server / nat_type echoed. `failure`
/// stays at its default 0 = ID_NOT_EXIST: clients tell success by a non-empty socket_addr.
#[tokio::test]
async fn punch_hole_sent_by_target_reaches_controller_as_punch_hole_response() {
    const TARGET: &str = "ph-sent-response";
    // Arrange
    let server = TestServer::start().await;
    let (mut target, keys) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut target_tcp = server.tcp().await;

    // Act
    let sent = punch_hole_sent(&ph.socket_addr, TARGET, TARGET_RELAY);
    target_tcp.send(&rv(|m| m.set_punch_hole_sent(sent))).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);

    // Assert
    assert_eq!(decode_addr(&res.socket_addr), target_tcp.local_addr());
    assert_eq!(
        open_id_pk(&res.pk, &server.public_key),
        signed_pk(TARGET, &keys.pk)
    );
    assert_eq!(res.relay_server, TARGET_RELAY);
    assert_eq!(res.nat_type(), NatType::SYMMETRIC);
    assert_eq!(res.failure.enum_value(), Ok(ID_NOT_EXIST));
    server.stop().await;
}

/// rs:778-799 (`same_intranet`: same IPv4 address, not websocket) and rs:578-580, 690-720: a
/// controller on the target's IP gets FetchLocalAddr forwarded instead, and the target's
/// LocalAddr comes back as PunchHoleResponse{is_local} with the target's LAN address.
#[tokio::test]
async fn punch_hole_request_from_targets_ip_goes_through_fetch_local_addr() {
    const TARGET: &str = "ph-fetch-local-addr";
    // Arrange
    let server = TestServer::start().await;
    let (mut target, keys) = server.online_peer(TARGET).await;
    let mut controller =
        TcpClient::connect_from(target.local_addr().ip(), server.main_addr()).await;
    let lan: SocketAddr = "192.168.7.20:21118".parse().unwrap();

    // Act
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let fla = expect_msg!(target.recv().await, FetchLocalAddr);
    let la = local_addr(&fla.socket_addr, TARGET, lan);
    let mut target_tcp = server.tcp().await;
    target_tcp.send(&rv(|m| m.set_local_addr(la.clone()))).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);

    // Assert
    assert_eq!(decode_addr(&fla.socket_addr), controller.local_addr());
    let expected = FetchLocalAddr {
        socket_addr: fla.socket_addr.clone(),
        ..Default::default()
    };
    assert_eq!(fla, expected);
    assert_eq!(decode_addr(&res.socket_addr), lan);
    assert!(res.is_local());
    assert_eq!(res.relay_server, la.relay_server);
    assert_eq!(
        open_id_pk(&res.pk, &server.public_key),
        signed_pk(TARGET, &keys.pk)
    );
    server.stop().await;
}

/// rs:745, 814-820, 900: an id hbbs does not know, including one that only sent RegisterPeer
/// (no pk, nothing stored: rs:641-643), gets PunchHoleResponse{ID_NOT_EXIST} on the
/// controller's own connection.
#[tokio::test]
async fn punch_hole_request_to_unknown_id_gets_id_not_exist() {
    // Arrange
    let server = TestServer::start().await;
    let mut pk_less = server.udp().await;
    assert!(pk_less.register_peer("peer-without-pk").await.request_pk);

    // Act + Assert
    for id in ["unknown-peer", "peer-without-pk"] {
        let mut controller = server.tcp().await;
        controller.punch_hole_request(server.punch_req(id)).await;
        let res = expect_msg!(controller.recv().await, PunchHoleResponse);
        let expected = PunchHoleResponse {
            failure: ID_NOT_EXIST.into(),
            ..Default::default()
        };
        assert_eq!(res, expected, "{id}");
    }
    pk_less.assert_silent().await;
    server.stop().await;
}

/// rs:730-738: a licence key other than the server's (wrong or missing) gets
/// PunchHoleResponse{LICENSE_MISMATCH} before any peer lookup; the online target hears nothing.
#[tokio::test]
async fn punch_hole_request_with_wrong_or_missing_licence_key_gets_license_mismatch() {
    const TARGET: &str = "ph-licence-mismatch";
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;

    // Act + Assert
    for key in ["wrong-licence-key", ""] {
        let mut controller = server.tcp().await;
        let req = PunchHoleRequest {
            licence_key: key.to_owned(),
            ..server.punch_req(TARGET)
        };
        controller.punch_hole_request(req).await;
        let res = expect_msg!(controller.recv().await, PunchHoleResponse);
        assert_eq!(
            res.failure.enum_value(),
            Ok(LICENSE_MISMATCH),
            "key {key:?}"
        );
    }
    target.assert_silent().await;
    server.stop().await;
}

/// rs:475-484: PunchHoleRequest, PunchHoleSent and LocalAddr over UDP are ignored (no UDP
/// reflection / amplification): neither the sender nor the target hears anything, and the
/// server keeps answering.
#[tokio::test]
async fn punch_messages_over_udp_get_no_answer() {
    const TARGET: &str = "ph-udp-ignored";
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut sender = server.udp().await;
    let victim = encode_addr(target.local_addr());
    let lan: SocketAddr = "192.168.7.20:21118".parse().unwrap();
    let msgs = [
        rv(|m| m.set_punch_hole_request(server.punch_req(TARGET))),
        rv(|m| m.set_punch_hole_sent(punch_hole_sent(&victim, TARGET, TARGET_RELAY))),
        rv(|m| m.set_local_addr(local_addr(&victim, TARGET, lan))),
    ];

    // Act
    for msg in &msgs {
        sender.send(msg).await;
    }

    // Assert
    sender.assert_silent().await;
    target.assert_silent().await;
    assert!(sender.register_peer("still-serving").await.request_pk);
    server.stop().await;
}
