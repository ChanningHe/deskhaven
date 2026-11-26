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

#[tokio::test]
async fn punch_hole_request_to_online_peer_is_forwarded_as_punch_hole_with_controller_addr() {
    const TARGET: &str = "ph-request-forwarded";
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;

    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);

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

/// `failure` stays at its default 0 = ID_NOT_EXIST: clients tell success by a non-empty
/// socket_addr.
#[tokio::test]
async fn punch_hole_sent_by_target_reaches_controller_as_punch_hole_response() {
    const TARGET: &str = "ph-sent-response";
    let server = TestServer::start().await;
    let (mut target, keys) = server.online_peer(TARGET).await;
    let mut controller = server.tcp().await;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut target_tcp = server.tcp().await;

    let sent = punch_hole_sent(&ph.socket_addr, TARGET, TARGET_RELAY);
    target_tcp.send(&rv(|m| m.set_punch_hole_sent(sent))).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);

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

#[tokio::test]
async fn punch_hole_request_from_targets_ip_goes_through_fetch_local_addr() {
    const TARGET: &str = "ph-fetch-local-addr";
    let server = TestServer::start().await;
    let (mut target, keys) = server.online_peer(TARGET).await;
    let mut controller =
        TcpClient::connect_from(target.local_addr().ip(), server.main_addr()).await;
    let lan: SocketAddr = "192.168.7.20:21118".parse().unwrap();

    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let fla = expect_msg!(target.recv().await, FetchLocalAddr);
    let la = local_addr(&fla.socket_addr, TARGET, lan);
    let mut target_tcp = server.tcp().await;
    target_tcp.send(&rv(|m| m.set_local_addr(la.clone()))).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);

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

#[tokio::test]
async fn punch_hole_request_to_unknown_id_gets_id_not_exist() {
    let server = TestServer::start().await;
    let mut pk_less = server.udp().await;
    assert!(pk_less.register_peer("peer-without-pk").await.request_pk);

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

#[tokio::test]
async fn punch_hole_request_with_wrong_or_missing_licence_key_gets_license_mismatch() {
    const TARGET: &str = "ph-licence-mismatch";
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;

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

/// No UDP reflection or amplification: these requests are ignored over UDP.
#[tokio::test]
async fn punch_messages_over_udp_get_no_answer() {
    const TARGET: &str = "ph-udp-ignored";
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

    for msg in &msgs {
        sender.send(msg).await;
    }

    sender.assert_silent().await;
    target.assert_silent().await;
    assert!(sender.register_peer("still-serving").await.request_pk);
    server.stop().await;
}
