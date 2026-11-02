//! Upstream-behavior baseline (M1-04): the websocket port (main port + 2) speaks the same
//! rendezvous protocol as the TCP main port, one protobuf per binary message.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit,
//! `handle_listener_inner` rs:1184-1243 with `ws = true`).

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::tokio;
use std::net::{IpAddr, SocketAddr};
use support::{
    Conn, Rendezvous, TestServer, WsClient, decode_addr, expect_msg, next_client_ip,
    punch_hole_sent, rv,
};

const TARGET: &str = "ws-target-peer";

/// rs:1218-1227, 581-595: TestNatRequest over websocket gets TestNatResponse with the
/// client's source port.
#[tokio::test]
async fn websocket_answers_test_nat_with_client_port() {
    // Arrange
    let server = TestServer::start().await;
    let mut ws = server.ws().await;

    // Act
    let res = ws.test_nat().await;

    // Assert
    assert_eq!(res.port, i32::from(ws.local_addr().port()));
    server.stop().await;
}

/// rs:778 (`!ws`): over websocket a PunchHoleRequest is forwarded as PunchHole even from the
/// target's own IP (never FetchLocalAddr), and the target's PunchHoleSent comes back over the
/// websocket as PunchHoleResponse (the ws sink is stored in `tcp_punch`, rs:531-536).
#[tokio::test]
async fn websocket_punch_hole_round_trip_never_fetches_local_addr() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let target_ip = target.local_addr().ip();
    let mut controller = WsClient::connect_from(target_ip, server.ws_addr(), &[]).await;

    // Act
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut target_tcp = server.tcp().await;
    let sent = punch_hole_sent(&ph.socket_addr, TARGET, "");
    target_tcp.send(&rv(|m| m.set_punch_hole_sent(sent))).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);

    // Assert
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    assert_eq!(decode_addr(&res.socket_addr), target_tcp.local_addr());
    server.stop().await;
}

/// rs:1196-1215: `X-Real-IP`, else `X-Forwarded-For`, is trusted as-is and replaces the
/// client address with `<ip>:0`: TestNat reports port 0 and the PunchHole carries the claimed
/// address. M1-08 gives websocket connections a unique key instead of `ip:0`.
#[tokio::test]
async fn websocket_client_addr_is_proxy_header_ip_port_0_upstream_behavior_m1_08_changes_this() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let cases = [
        ("X-Real-IP", "203.0.113.9"),
        ("X-Forwarded-For", "203.0.113.10"),
    ];

    for (header, ip) in cases {
        let claimed = SocketAddr::new(ip.parse::<IpAddr>().unwrap(), 0);
        let headers = [(header, ip)];
        let mut nat = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;
        let mut controller =
            WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;

        // Act
        let port = nat.test_nat().await.port;
        controller
            .punch_hole_request(server.punch_req(TARGET))
            .await;
        let ph = expect_msg!(target.recv().await, PunchHole);

        // Assert
        assert_eq!(port, 0, "{header}");
        assert_eq!(decode_addr(&ph.socket_addr), claimed, "{header}");
    }
    server.stop().await;
}
