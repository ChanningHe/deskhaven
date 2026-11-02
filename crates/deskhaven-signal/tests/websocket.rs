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

/// Where the server's idea of a websocket client's address comes from.
enum Expect {
    /// `<ip>:0` taken from a proxy header.
    Claimed(&'static str),
    /// The real TCP peer address (header absent or unparsable).
    Real,
}

/// rs:1196-1215: `X-Real-IP`, else `X-Forwarded-For`, is trusted as-is and replaces the client
/// address with `<ip>:0` (`[<ip>]:0` for IPv6), so TestNat reports port 0 and the PunchHole
/// carries the claimed address. `X-Real-IP` wins when both are present; a value that does not
/// parse as a single IP (e.g. the multi-hop form `a, b`) silently falls back to the real
/// address. M1-08 gives websocket connections a unique key instead of `ip:0`.
#[tokio::test]
async fn websocket_client_addr_is_proxy_header_ip_port_0_upstream_behavior_m1_08_changes_this() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let cases: [(&[(&str, &str)], Expect); 5] = [
        (
            &[("X-Real-IP", "203.0.113.9")],
            Expect::Claimed("203.0.113.9"),
        ),
        (
            &[("X-Forwarded-For", "203.0.113.10")],
            Expect::Claimed("203.0.113.10"),
        ),
        (
            &[
                ("X-Forwarded-For", "203.0.113.11"),
                ("X-Real-IP", "203.0.113.12"),
            ],
            Expect::Claimed("203.0.113.12"),
        ),
        (
            &[("X-Real-IP", "2001:db8::5")],
            Expect::Claimed("2001:db8::5"),
        ),
        (
            &[("X-Forwarded-For", "203.0.113.13, 10.0.0.1")],
            Expect::Real,
        ),
    ];

    for (headers, expect) in cases {
        let mut nat = WsClient::connect_from(next_client_ip(), server.ws_addr(), headers).await;
        let mut controller =
            WsClient::connect_from(next_client_ip(), server.ws_addr(), headers).await;
        let (nat_addr, controller_addr) = match expect {
            Expect::Claimed(ip) => {
                let claimed = SocketAddr::new(ip.parse::<IpAddr>().unwrap(), 0);
                (claimed, claimed)
            }
            Expect::Real => (nat.local_addr(), controller.local_addr()),
        };

        // Act
        let port = nat.test_nat().await.port;
        controller
            .punch_hole_request(server.punch_req(TARGET))
            .await;
        let ph = expect_msg!(target.recv().await, PunchHole);

        // Assert
        assert_eq!(port, i32::from(nat_addr.port()), "{headers:?}");
        assert_eq!(decode_addr(&ph.socket_addr), controller_addr, "{headers:?}");
    }
    server.stop().await;
}

/// rs:531-536 keys `tcp_punch` by the client address, which for websocket clients behind one
/// proxy IP is the same `<ip>:0`: the second controller's sink replaces the first's, so the
/// target's answer for the first goes to the second and the first never hears back. M1-08
/// gives websocket connections a unique key.
#[tokio::test]
async fn websocket_controllers_behind_one_proxy_ip_overwrite_each_other_upstream_behavior_m1_08_changes_this()
 {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let headers = [("X-Real-IP", "203.0.113.20")];
    let mut first = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;
    let mut second = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;

    // Act: the first request is forwarded before the second is sent.
    first.punch_hole_request(server.punch_req(TARGET)).await;
    let ph_first = expect_msg!(target.recv().await, PunchHole);
    second.punch_hole_request(server.punch_req(TARGET)).await;
    let ph_second = expect_msg!(target.recv().await, PunchHole);
    let mut target_tcp = server.tcp().await;
    let sent = punch_hole_sent(&ph_first.socket_addr, TARGET, "");
    target_tcp.send(&rv(|m| m.set_punch_hole_sent(sent))).await;

    // Assert
    assert_eq!(
        decode_addr(&ph_first.socket_addr),
        decode_addr(&ph_second.socket_addr),
        "both controllers look like the same address"
    );
    expect_msg!(second.recv().await, PunchHoleResponse);
    first.assert_silent().await;
    server.stop().await;
}
