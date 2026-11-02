//! Upstream-behavior baseline (M1-04): the websocket port (main port + 2) speaks the same
//! rendezvous protocol as the TCP main port, one protobuf per binary message.
//!
//! Pins upstream hbbs at 873230e (`rs` = src/rendezvous_server.rs at that commit,
//! `handle_listener_inner` rs:1184-1243 with `ws = true`).

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{rendezvous_proto::register_pk_response::Result::OK, tokio};
use std::net::IpAddr;
use support::{
    Conn, PeerKeys, Rendezvous, TestServer, WsClient, decode_addr, expect_msg, next_client_ip,
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
    /// The IP taken from a proxy header, with a port of the server's choosing.
    Claimed(&'static str),
    /// The real TCP peer address (header absent or unparsable).
    Real,
}

/// Changed in M1-08. rs:1196-1215: `X-Real-IP`, else `X-Forwarded-For`, is trusted as-is and
/// replaces the client's IP (`X-Real-IP` wins when both are present; a value that does not parse
/// as a single IP, e.g. the multi-hop form `a, b`, silently falls back to the real address).
/// Upstream also set the port to 0, so every connection claiming one IP had the same address;
/// now each connection gets its own non-zero port, which TestNat reports and the PunchHole
/// carries.
#[tokio::test]
async fn websocket_client_addr_takes_the_proxy_header_ip_with_a_unique_port() {
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

        // Act
        let port = nat.test_nat().await.port;
        controller
            .punch_hole_request(server.punch_req(TARGET))
            .await;
        let ph = expect_msg!(target.recv().await, PunchHole);

        // Assert
        let seen = decode_addr(&ph.socket_addr);
        match expect {
            Expect::Claimed(ip) => {
                let ip: IpAddr = ip.parse().unwrap();
                assert_eq!(seen.ip(), ip, "{headers:?}");
                assert_ne!(port, 0, "{headers:?}");
                assert_ne!(seen.port(), 0, "{headers:?}");
                assert_ne!(i32::from(seen.port()), port, "{headers:?}: shared port");
            }
            Expect::Real => {
                assert_eq!(port, i32::from(nat.local_addr().port()), "{headers:?}");
                assert_eq!(seen, controller.local_addr(), "{headers:?}");
            }
        }
    }
    server.stop().await;
}

/// Changed in M1-08. rs:531-536 keys `tcp_punch` by the client address, which upstream gave as
/// the same `<ip>:0` to every websocket client behind one proxy IP: the second controller's
/// sink replaced the first's, so the target's answer for the first went to the second. With a
/// port per connection each controller gets its own answer.
#[tokio::test]
async fn websocket_controllers_behind_one_proxy_ip_get_their_own_answers() {
    // Arrange
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let headers = [("X-Real-IP", "203.0.113.20")];
    let mut first = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;
    let mut second = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;
    first.punch_hole_request(server.punch_req(TARGET)).await;
    let ph_first = expect_msg!(target.recv().await, PunchHole);
    second.punch_hole_request(server.punch_req(TARGET)).await;
    let ph_second = expect_msg!(target.recv().await, PunchHole);

    // Act: one answer per connection, as clients send them.
    for ph in [&ph_first, &ph_second] {
        let sent = punch_hole_sent(&ph.socket_addr, TARGET, "");
        let mut target_tcp = server.tcp().await;
        target_tcp.send(&rv(|m| m.set_punch_hole_sent(sent))).await;
    }

    // Assert
    assert_ne!(
        decode_addr(&ph_first.socket_addr),
        decode_addr(&ph_second.socket_addr)
    );
    expect_msg!(first.recv().await, PunchHoleResponse);
    expect_msg!(second.recv().await, PunchHoleResponse);
    first.assert_silent().await;
    second.assert_silent().await;
    server.stop().await;
}

/// M1-08: two peers registered over websocket behind one proxy IP each get their own punch
/// holes (registration over websocket and per-connection addresses together).
#[tokio::test]
async fn websocket_peers_behind_one_proxy_ip_each_get_their_punch_holes() {
    // Arrange
    let server = TestServer::start().await;
    let headers = [("X-Real-IP", "203.0.113.30")];
    let mut peers = Vec::new();
    for id in ["ws-proxied-peer-1", "ws-proxied-peer-2"] {
        let mut peer = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;
        assert_eq!(peer.register_pk(id, &PeerKeys::random()).await, OK);
        peers.push((id, peer));
    }

    for (id, peer) in &mut peers {
        // Act
        let mut controller = server.tcp().await;
        controller.punch_hole_request(server.punch_req(id)).await;

        // Assert
        let ph = expect_msg!(peer.recv().await, PunchHole);
        assert_eq!(
            decode_addr(&ph.socket_addr),
            controller.local_addr(),
            "{id}"
        );
    }
    for (_, peer) in &mut peers {
        peer.assert_silent().await;
    }
    server.stop().await;
}
