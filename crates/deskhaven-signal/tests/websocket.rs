// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use hbb_common::{
    rendezvous_proto::register_pk_response::Result::{OK, TOO_FREQUENT, UUID_MISMATCH},
    tokio,
};
use std::net::IpAddr;
use support::{
    Conn, PeerKeys, Rendezvous, TestServer, WsClient, decode_addr, expect_msg, next_client_ip,
    punch_hole_sent, rv, trusting,
};

const TARGET: &str = "ws-target-peer";

#[tokio::test]
async fn websocket_answers_test_nat_with_client_port() {
    let server = TestServer::start().await;
    let mut ws = server.ws().await;

    let res = ws.test_nat().await;

    assert_eq!(res.port, i32::from(ws.local_addr().port()));
    server.stop().await;
}

#[tokio::test]
async fn websocket_punch_hole_round_trip_never_fetches_local_addr() {
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let target_ip = target.local_addr().ip();
    let mut controller = WsClient::connect_from(target_ip, server.ws_addr(), &[]).await;

    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);
    let mut target_tcp = server.tcp().await;
    let sent = punch_hole_sent(&ph.socket_addr, TARGET, "");
    target_tcp.send(&rv(|m| m.set_punch_hole_sent(sent))).await;
    let res = expect_msg!(controller.recv().await, PunchHoleResponse);

    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    assert_eq!(decode_addr(&res.socket_addr), target_tcp.local_addr());
    server.stop().await;
}

type Headers = &'static [(&'static str, &'static str)];

enum Expect {
    Claimed(&'static str),
    Real,
}

/// Only a trusted proxy's `X-Forwarded-For` counts (`X-Real-IP` is not used), and each
/// connection claiming an IP gets its own non-zero port.
#[tokio::test]
async fn websocket_client_addr_takes_a_trusted_proxys_header_ip_with_a_unique_port() {
    let proxy = next_client_ip();
    let server = TestServer::start_with(trusting(&[proxy])).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let cases: [(IpAddr, Headers, Expect); 5] = [
        (
            proxy,
            &[("X-Forwarded-For", "203.0.113.10")],
            Expect::Claimed("203.0.113.10"),
        ),
        (
            proxy,
            &[("X-Forwarded-For", "2001:db8::5")],
            Expect::Claimed("2001:db8::5"),
        ),
        (proxy, &[("X-Real-IP", "203.0.113.9")], Expect::Real),
        (
            next_client_ip(),
            &[("X-Forwarded-For", "203.0.113.11")],
            Expect::Real,
        ),
        (proxy, &[], Expect::Real),
    ];

    for (from, headers, expect) in cases {
        let mut nat = WsClient::connect_from(from, server.ws_addr(), headers).await;
        let mut controller = WsClient::connect_from(from, server.ws_addr(), headers).await;

        let port = nat.test_nat().await.port;
        controller
            .punch_hole_request(server.punch_req(TARGET))
            .await;
        let ph = expect_msg!(target.recv().await, PunchHole);

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
                assert_eq!(
                    port,
                    i32::from(nat.local_addr().port()),
                    "{from} {headers:?}"
                );
                assert_eq!(seen, controller.local_addr(), "{from} {headers:?}");
            }
        }
    }
    server.stop().await;
}

#[tokio::test]
async fn untrusted_websocket_proxy_headers_are_ignored() {
    let server = TestServer::start().await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let headers = [
        ("X-Real-IP", "127.0.0.1"),
        ("X-Forwarded-For", "203.0.113.40"),
    ];
    let mut nat = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;
    let mut controller = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;

    let port = nat.test_nat().await.port;
    controller
        .punch_hole_request(server.punch_req(TARGET))
        .await;
    let ph = expect_msg!(target.recv().await, PunchHole);

    assert_eq!(port, i32::from(nat.local_addr().port()));
    assert_eq!(decode_addr(&ph.socket_addr), controller.local_addr());
    server.stop().await;
}

#[tokio::test]
async fn rotating_headers_from_an_untrusted_peer_do_not_bypass_the_per_ip_limit() {
    let server = TestServer::start().await;
    let source = next_client_ip();
    let mut limited_at = None;

    for n in 0..MAX_TRIES {
        let fake = format!("198.51.100.{}", n + 1);
        let headers = [
            ("X-Forwarded-For", fake.as_str()),
            ("X-Real-IP", fake.as_str()),
        ];
        let mut ws = WsClient::connect_from(source, server.ws_addr(), &headers).await;
        let res = ws
            .register_pk(&format!("rotating-{n:03}"), &PeerKeys::random())
            .await;
        if res == TOO_FREQUENT {
            limited_at = Some(n);
            break;
        }
        assert_eq!(res, OK, "rotating-{n:03}");
    }

    assert!(limited_at.is_some(), "never limited in {MAX_TRIES} tries");
    server.stop().await;
}

/// Registrations tried before the per-IP limit must have hit (it allows about 30 a minute).
const MAX_TRIES: usize = 40;

#[tokio::test]
async fn untrusted_header_cannot_claim_a_devices_ip_to_change_its_pk() {
    const ID: &str = "ws-hijack-target";
    let server = TestServer::start().await;
    let (mut device, keys) = server.online_peer(ID).await;
    let device_ip = device.local_addr().ip().to_string();
    let headers = [
        ("X-Forwarded-For", device_ip.as_str()),
        ("X-Real-IP", device_ip.as_str()),
    ];
    let mut attacker = WsClient::connect_from(next_client_ip(), server.ws_addr(), &headers).await;

    let res = attacker.register_pk(ID, &keys.with_new_pk()).await;

    assert_eq!(res, UUID_MISMATCH);
    let mut controller = server.tcp().await;
    controller.punch_hole_request(server.punch_req(ID)).await;
    expect_msg!(device.recv().await, PunchHole);
    attacker.assert_silent().await;
    server.stop().await;
}

#[tokio::test]
async fn websocket_controllers_behind_one_proxy_ip_get_their_own_answers() {
    let proxy = next_client_ip();
    let server = TestServer::start_with(trusting(&[proxy])).await;
    let (mut target, _) = server.online_peer(TARGET).await;
    let headers = [("X-Forwarded-For", "203.0.113.20")];
    let mut first = WsClient::connect_from(proxy, server.ws_addr(), &headers).await;
    let mut second = WsClient::connect_from(proxy, server.ws_addr(), &headers).await;
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

    let (a, b) = (
        decode_addr(&ph_first.socket_addr),
        decode_addr(&ph_second.socket_addr),
    );
    assert_eq!(a.ip(), "203.0.113.20".parse::<IpAddr>().unwrap());
    assert_ne!(a, b);
    expect_msg!(first.recv().await, PunchHoleResponse);
    expect_msg!(second.recv().await, PunchHoleResponse);
    first.assert_silent().await;
    second.assert_silent().await;
    server.stop().await;
}

#[tokio::test]
async fn websocket_peers_behind_one_proxy_ip_each_get_their_punch_holes() {
    let proxy = next_client_ip();
    let server = TestServer::start_with(trusting(&[proxy])).await;
    let headers = [("X-Forwarded-For", "203.0.113.30")];
    let mut peers = Vec::new();
    for id in ["ws-proxied-peer-1", "ws-proxied-peer-2"] {
        let mut peer = WsClient::connect_from(proxy, server.ws_addr(), &headers).await;
        assert_eq!(peer.register_pk(id, &PeerKeys::random()).await, OK);
        peers.push((id, peer));
    }

    for (id, peer) in &mut peers {
        let mut controller = server.tcp().await;
        controller.punch_hole_request(server.punch_req(id)).await;

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
