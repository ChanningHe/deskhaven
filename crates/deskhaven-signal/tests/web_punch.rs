//! In-process web punches (s4): `SignalHandle::web_punch` asks a device for a relay session
//! (and a WebRTC answer) on behalf of a browser. Wire behavior follows rustdesk-server PR #734
//! (39b1b3f); WebRTC routes are only ever opened here, never by wire clients.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{SignalConfig, WEB_PUNCH_TIMEOUT, WebPunchError};
use hbb_common::{
    rendezvous_proto::{NatType, PunchHoleRequest},
    tokio::{self, time::Duration},
};
use support::{
    ANSWER, BROWSER_IP, Conn, FOREIGN_RELAY, PeerKeys, RELAY_UUID, Rendezvous, TcpClient,
    TestServer, WEB_RELAY, answered_punch, assert_route_gone, decode_addr, expect_msg, local_addr,
    offer, online_bits, open_id_pk, punch, punch_hole_sent, relay_answer, rv, send_relay_response,
    web_server,
};

/// Relay-only (no offer): the device gets a PunchHole that forces relay to the deployment's
/// relay, with the browser's address and no offer; its RelayResponse (UDP-registered device)
/// comes back as the session's uuid and relay plus the device key signed by the server.
#[tokio::test]
async fn relay_only_punch_returns_uuid_relay_and_the_signed_device_key() {
    const TARGET: &str = "web-relay-only";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, keys) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();

    // Act
    let (ph, res) = answered_punch(&server, &mut peer, ip, TARGET, None, WEB_RELAY, "").await;

    // Assert
    assert_eq!(ph.nat_type.enum_value(), Ok(NatType::SYMMETRIC));
    assert_eq!(ph.relay_server, WEB_RELAY);
    assert_eq!(ph.webrtc_sdp_offer, "");
    assert_eq!(decode_addr(&ph.socket_addr).ip(), BROWSER_IP);
    let route = res.unwrap();
    assert_eq!(route.uuid, RELAY_UUID);
    assert_eq!(route.relay_server, WEB_RELAY);
    assert_eq!(route.answer, None);
    let id_pk = open_id_pk(&route.signed_pk, &server.public_key);
    assert_eq!(id_pk.id, TARGET);
    assert_eq!(id_pk.pk, keys.pk);
    server.stop().await;
}

/// With an offer, a device registered over TCP gets it in the PunchHole on its connection and
/// its WebRTC answer is passed through.
#[tokio::test]
async fn offer_reaches_a_tcp_registered_device_and_its_answer_comes_back() {
    const TARGET: &str = "web-offer-tcp";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let mut peer = server.tcp().await;
    assert_eq!(
        peer.register_pk(TARGET, &PeerKeys::random()).await,
        hbb_common::rendezvous_proto::register_pk_response::Result::OK
    );
    let ip = peer.local_addr().ip();

    // Act
    let (ph, res) = answered_punch(
        &server,
        &mut peer,
        ip,
        TARGET,
        Some(offer()),
        WEB_RELAY,
        ANSWER,
    )
    .await;

    // Assert
    assert_eq!(ph.webrtc_sdp_offer, offer());
    assert_eq!(ph.nat_type.enum_value(), Ok(NatType::SYMMETRIC));
    let route = res.unwrap();
    assert_eq!(route.answer.as_deref(), Some(ANSWER));
    assert_eq!(route.uuid, RELAY_UUID);
    server.stop().await;
}

/// `always_use_relay` is the administrator's policy: the offer is not sent, and an answer the
/// device sends anyway is not passed on.
#[tokio::test]
async fn always_use_relay_withholds_the_offer() {
    const TARGET: &str = "web-always-relay";
    // Arrange
    let server = web_server(|cfg| SignalConfig {
        always_use_relay: true,
        ..cfg
    })
    .await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();

    // Act
    let (ph, res) = answered_punch(
        &server,
        &mut peer,
        ip,
        TARGET,
        Some(offer()),
        WEB_RELAY,
        ANSWER,
    )
    .await;

    // Assert
    assert_eq!(ph.webrtc_sdp_offer, "");
    let route = res.unwrap();
    assert_eq!(route.answer, None);
    assert_eq!(route.relay_server, WEB_RELAY);
    server.stop().await;
}

/// A device with its own `relay-server` option answers with that relay, which the browser
/// cannot reach through the deployment's `/ws/relay`.
#[tokio::test]
async fn a_relay_other_than_the_one_sent_is_foreign() {
    const TARGET: &str = "web-foreign-relay";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();

    // Act
    let (_, res) = answered_punch(&server, &mut peer, ip, TARGET, None, FOREIGN_RELAY, "").await;

    // Assert
    assert_eq!(
        res.unwrap_err(),
        WebPunchError::ForeignRelay {
            relay_server: FOREIGN_RELAY.to_owned()
        }
    );
    server.stop().await;
}

/// The LAN rewrite of RelayResponse.relay_server (`mask` / `local_ip`) is for wire controllers
/// on the LAN; it never applies to a web punch, whose browser reaches the relay through the
/// deployment.
#[tokio::test]
async fn the_lan_relay_rewrite_does_not_touch_a_web_punch() {
    const TARGET: &str = "web-lan-browser";
    // Arrange: the browser's IP is inside the LAN mask.
    let server = web_server(|cfg| SignalConfig {
        mask: Some("198.51.100.0/24".parse().unwrap()),
        local_ip: Some("10.0.0.1".to_owned()),
        ..cfg
    })
    .await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();

    // Act
    let (_, res) = answered_punch(&server, &mut peer, ip, TARGET, None, WEB_RELAY, "").await;

    // Assert
    assert_eq!(res.unwrap().relay_server, WEB_RELAY);
    server.stop().await;
}

#[tokio::test]
async fn without_a_relay_nothing_is_sent() {
    const TARGET: &str = "web-no-relay";
    // Arrange
    let server = TestServer::start().await;
    let (mut peer, _) = server.online_peer(TARGET).await;

    // Act
    let res = punch(&server, TARGET, None).await.unwrap();

    // Assert
    assert_eq!(res.unwrap_err(), WebPunchError::NoRelay);
    peer.assert_silent().await;
    server.stop().await;
}

#[tokio::test]
async fn an_unknown_device_is_not_found() {
    // Arrange
    let server = web_server(|cfg| cfg).await;

    // Act
    let res = punch(&server, "web-unknown", None).await.unwrap();

    // Assert
    assert_eq!(res.unwrap_err(), WebPunchError::NotFound);
    server.stop().await;
}

/// A device whose TCP channel closed is offline at once (no REG_TIMEOUT wait).
#[tokio::test]
async fn an_offline_device_is_offline() {
    const TARGET: &str = "web-offline";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let mut peer = server.tcp().await;
    peer.register_pk(TARGET, &PeerKeys::random()).await;
    drop(peer);
    let deadline = tokio::time::Instant::now() + support::STEP;
    loop {
        let res = server.nat().await.online_request(&[TARGET]).await;
        if !online_bits(&res.states, 1)[0] {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "still online");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Act
    let res = punch(&server, TARGET, None).await.unwrap();

    // Assert
    assert_eq!(res.unwrap_err(), WebPunchError::Offline);
    server.stop().await;
}

#[tokio::test]
async fn an_invalid_offer_is_refused_before_anything_is_sent() {
    const TARGET: &str = "web-bad-offer";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let oversized = format!("webrtc://{}", "A".repeat(4096));

    for bad in [
        "not-an-endpoint".to_owned(),
        "webrtc://!!".to_owned(),
        oversized,
    ] {
        // Act
        let res = punch(&server, TARGET, Some(bad)).await.unwrap();

        // Assert
        assert_eq!(res.unwrap_err(), WebPunchError::InvalidOffer);
    }
    peer.assert_silent().await;
    server.stop().await;
}

/// A device that never answers: `NoAnswer` after WEB_PUNCH_TIMEOUT, and a late answer goes
/// nowhere. Paused clock.
#[tokio::test(start_paused = true)]
async fn a_device_that_does_not_answer_times_out() {
    const TARGET: &str = "web-no-answer";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let task = punch(&server, TARGET, None);
    let ph = expect_msg!(peer.recv().await, PunchHole);

    // Act
    tokio::time::advance(WEB_PUNCH_TIMEOUT + Duration::from_secs(1)).await;

    // Assert
    assert_eq!(task.await.unwrap().unwrap_err(), WebPunchError::NoAnswer);
    let ip = peer.local_addr().ip();
    send_relay_response(&server, ip, relay_answer(&ph, TARGET, WEB_RELAY, "")).await;
    server.stop().await;
}

/// Only the punched device, from the IP it registered from, can answer a web punch, and only
/// with a RelayResponse: one naming another id or sent from another host, and a PunchHoleSent
/// or LocalAddr for the punch's address, are dropped and do not use the punch up.
#[tokio::test]
async fn forged_answers_are_dropped_and_the_real_one_still_lands() {
    const TARGET: &str = "web-forged-answer";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, keys) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();
    let task = punch(&server, TARGET, None);
    let ph = expect_msg!(peer.recv().await, PunchHole);

    // Act
    let mut forged = relay_answer(&ph, TARGET, WEB_RELAY, "");
    forged.uuid = "forged-uuid".to_owned();
    send_relay_response(&server, support::next_client_ip(), forged.clone()).await;
    forged.set_id("web-someone-else".to_owned());
    send_relay_response(&server, ip, forged).await;
    let mut other_answers = TcpClient::connect_from(ip, server.main_addr()).await;
    let sent = punch_hole_sent(&ph.socket_addr, TARGET, WEB_RELAY);
    other_answers
        .send(&rv(|m| m.set_punch_hole_sent(sent)))
        .await;
    let lan = local_addr(&ph.socket_addr, TARGET, "10.0.0.9:21118".parse().unwrap());
    other_answers.send(&rv(|m| m.set_local_addr(lan))).await;
    tokio::time::sleep(support::QUIET).await;
    send_relay_response(&server, ip, relay_answer(&ph, TARGET, WEB_RELAY, "")).await;

    // Assert
    let route = task.await.unwrap().unwrap();
    assert_eq!(route.uuid, RELAY_UUID);
    assert_eq!(open_id_pk(&route.signed_pk, &server.public_key).pk, keys.pk);
    server.stop().await;
}

/// Regression: wire controllers still cannot start WebRTC. A PunchHoleRequest's offer is not
/// forwarded, and a WebRTC answer in a RelayResponse for a wire controller is not passed on.
#[tokio::test]
async fn wire_offers_and_answers_are_still_dropped() {
    const TARGET: &str = "web-wire-regression";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();
    let mut controller = server.tcp().await;
    let req = PunchHoleRequest {
        webrtc_sdp_offer: offer(),
        ..server.punch_req(TARGET)
    };

    // Act
    controller.punch_hole_request(req).await;
    let ph = expect_msg!(peer.recv().await, PunchHole);
    send_relay_response(&server, ip, relay_answer(&ph, TARGET, WEB_RELAY, ANSWER)).await;

    // Assert
    assert_eq!(ph.webrtc_sdp_offer, "");
    let rr = expect_msg!(controller.recv().await, RelayResponse);
    assert_eq!(rr.uuid, RELAY_UUID);
    assert_eq!(rr.webrtc_sdp_answer, "");
    server.stop().await;
}

/// An offer that gets no usable WebRTC answer (none, or over 4096 bytes) leaves a relay-only
/// route: no answer, no candidates, and the route is revoked at once (its per-IP slot freed).
#[tokio::test]
async fn an_offer_without_a_usable_answer_revokes_the_route() {
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let oversized = format!("webrtc://{}", "a".repeat(4096));

    for (target, answer) in [
        ("web-no-webrtc", ""),
        ("web-huge-answer", oversized.as_str()),
    ] {
        let (mut peer, _) = server.online_peer(target).await;
        let ip = peer.local_addr().ip();

        // Act
        let (ph, res) = answered_punch(
            &server,
            &mut peer,
            ip,
            target,
            Some(offer()),
            WEB_RELAY,
            answer,
        )
        .await;

        // Assert
        let route = res.unwrap();
        assert_eq!(route.answer, None, "{target}");
        assert_eq!(route.uuid, RELAY_UUID);
        let closed = route.send_candidate("c".to_owned());
        assert_eq!(closed, Err(deskhaven_signal::CandidateError::Closed));
        assert_route_gone(&server, ip, &ph.socket_addr).await;
    }
    server.stop().await;
}
