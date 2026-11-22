//! Trickle ICE of web punch routes (s4): while the WebRTC session of a `web_punch` lives, the
//! device's candidates reach the `WebRoute` and the route's candidates reach the device. The
//! limits follow rustdesk-server PR #734 (39b1b3f): candidate 2048 B, session key 256 B, 128
//! candidates per route and direction, route TTL 60 s, 256 candidate frames and 16 misses per
//! connection.

// Test code: panicking helpers are fine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use deskhaven_signal::{CandidateError, WEB_ROUTE_TTL, WebPunch, WebPunchError, WebRoute};
use hbb_common::{
    bytes::Bytes,
    rendezvous_proto::{IceCandidate, register_pk_response},
    tokio::{self, time::Duration},
};
use support::{
    ANSWER, Conn, FINGERPRINT, PeerKeys, QUIET, Rendezvous, STEP, TcpClient, WEB_RELAY,
    answered_punch, assert_route_gone, candidate_conn, encode_addr, expect_msg, next_client_ip,
    offer, peer_candidate, punch, relay_answer, rv, send_relay_response, web_server, webrtc_route,
};

const MAX_FRAMES: usize = 256;
const BUDGET: usize = 128;

/// Whether `route` yields nothing within QUIET.
async fn assert_no_candidate(route: &mut WebRoute) {
    if let Ok(c) = tokio::time::timeout(QUIET, route.next_candidate()).await {
        panic!("unexpected candidate {c:?}");
    }
}

/// Sends `n` candidates that hit no route; the connection must survive exactly MAX_MISSES.
async fn send_misses(conn: &mut TcpClient, ctl: &Bytes, n: usize) {
    for i in 0..n {
        conn.send(&peer_candidate(ctl, "sha-256 00:00", &format!("miss-{i}")))
            .await;
    }
}

/// Candidates the device sends (even before its RelayResponse is in) reach the route, and the
/// route's candidates reach the device over UDP with the offer's session key.
#[tokio::test]
async fn candidates_flow_both_ways_for_a_udp_registered_device() {
    const TARGET: &str = "ice-udp-device";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();
    let task = punch(&server, TARGET, Some(offer()));
    let ph = expect_msg!(peer.recv().await, PunchHole);
    let mut ice = candidate_conn(&server, ip).await;

    // Act
    ice.send(&peer_candidate(&ph.socket_addr, FINGERPRINT, "early"))
        .await;
    send_relay_response(&server, ip, relay_answer(&ph, TARGET, WEB_RELAY, ANSWER)).await;
    let mut route = task.await.unwrap().unwrap();
    ice.send(&peer_candidate(&ph.socket_addr, FINGERPRINT, "late"))
        .await;
    route.send_candidate("to-device".to_owned()).unwrap();

    // Assert
    assert_eq!(route.next_candidate().await.as_deref(), Some("early"));
    assert_eq!(route.next_candidate().await.as_deref(), Some("late"));
    let got = expect_msg!(peer.recv().await, IceCandidate);
    let expected = IceCandidate {
        id: TARGET.to_owned(),
        session_key: FINGERPRINT.to_owned(),
        candidate: "to-device".to_owned(),
        ..Default::default()
    };
    assert_eq!(got, expected);
    server.stop().await;
}

/// A device registered over TCP gets the route's candidates on its registration connection.
#[tokio::test]
async fn candidates_reach_a_tcp_registered_device_on_its_connection() {
    const TARGET: &str = "ice-tcp-device";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let mut peer = server.tcp().await;
    let ok = register_pk_response::Result::OK;
    assert_eq!(peer.register_pk(TARGET, &PeerKeys::random()).await, ok);
    let ip = peer.local_addr().ip();
    let (_, res) = answered_punch(
        &server,
        &mut peer,
        ip,
        TARGET,
        Some(offer()),
        WEB_RELAY,
        ANSWER,
    )
    .await;
    let route = res.unwrap();

    // Act
    route.send_candidate("to-device".to_owned()).unwrap();

    // Assert
    let got = expect_msg!(peer.recv().await, IceCandidate);
    assert_eq!(
        (got.session_key.as_str(), got.candidate.as_str()),
        (FINGERPRINT, "to-device")
    );
    server.stop().await;
}

/// Candidates from another host, with another session key, for an address without a route,
/// or on a connection without a KeyExchange are dropped.
#[tokio::test]
async fn forged_candidates_are_dropped() {
    const TARGET: &str = "ice-forged";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, mut route) = webrtc_route(&server, TARGET).await;
    let ip = peer.local_addr().ip();
    let elsewhere = encode_addr("198.51.100.200:9".parse().unwrap());
    let mut plain = TcpClient::connect_from(ip, server.main_addr()).await;

    // Act
    let mut other_host = candidate_conn(&server, next_client_ip()).await;
    other_host
        .send(&peer_candidate(&ctl, FINGERPRINT, "other-host"))
        .await;
    let mut ice = candidate_conn(&server, ip).await;
    ice.send(&peer_candidate(&ctl, "sha-256 FF:FF", "wrong-key"))
        .await;
    ice.send(&peer_candidate(&elsewhere, FINGERPRINT, "no-route"))
        .await;
    plain
        .send(&peer_candidate(&ctl, FINGERPRINT, "plain"))
        .await;

    // Assert
    assert_no_candidate(&mut route).await;
    ice.send(&peer_candidate(&ctl, FINGERPRINT, "real")).await;
    assert_eq!(route.next_candidate().await.as_deref(), Some("real"));
    server.stop().await;
}

/// Oversized candidates and session keys are dropped and count as misses; a connection keeps
/// 16 misses and is closed at the 17th. Candidates from wire controllers (no address: the
/// controller's direction) never match: WebRTC routes are in-process only.
#[tokio::test]
async fn oversized_and_unmatched_candidates_are_misses_that_close_the_connection() {
    const TARGET: &str = "ice-misses";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, mut route) = webrtc_route(&server, TARGET).await;
    let mut ice = candidate_conn(&server, peer.local_addr().ip()).await;
    let long_candidate = "c".repeat(2049);
    let long_key = "k".repeat(257);
    let wire_controller = rv(|m| {
        m.set_ice_candidate(IceCandidate {
            id: TARGET.to_owned(),
            session_key: FINGERPRINT.to_owned(),
            candidate: "from-a-wire-controller".to_owned(),
            ..Default::default()
        })
    });

    // Act
    ice.send(&peer_candidate(&ctl, FINGERPRINT, &long_candidate))
        .await;
    ice.send(&peer_candidate(&ctl, &long_key, "long-key")).await;
    ice.send(&wire_controller).await;
    send_misses(&mut ice, &ctl, support::MAX_ICE_MISSES - 3).await;
    ice.send(&peer_candidate(&ctl, FINGERPRINT, &"c".repeat(2048)))
        .await;

    // Assert
    assert_eq!(route.next_candidate().await.map(|c| c.len()), Some(2048));
    send_misses(&mut ice, &ctl, 1).await;
    ice.assert_closed().await;
    server.stop().await;
}

/// 128 candidates per route and direction: the device's 129th is dropped quietly (not a miss)
/// and the route's 129th is refused; a connection carries at most 256 candidate frames.
#[tokio::test]
async fn each_direction_has_a_budget_and_a_connection_a_frame_cap() {
    const TARGET: &str = "ice-budget";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, mut route) = webrtc_route(&server, TARGET).await;
    let mut ice = candidate_conn(&server, peer.local_addr().ip()).await;

    // Act + Assert: device to route.
    for i in 0..=BUDGET {
        ice.send(&peer_candidate(&ctl, FINGERPRINT, &format!("c{i}")))
            .await;
    }
    for i in 0..BUDGET {
        let c = tokio::time::timeout(STEP, route.next_candidate()).await;
        assert_eq!(c.unwrap(), Some(format!("c{i}")));
    }
    assert_no_candidate(&mut route).await;
    for i in BUDGET + 1..MAX_FRAMES {
        ice.send(&peer_candidate(&ctl, FINGERPRINT, &format!("c{i}")))
            .await;
    }
    assert!(matches!(ice.recv_within(QUIET).await, support::Recv::Quiet));
    ice.send(&peer_candidate(&ctl, FINGERPRINT, "one-too-many"))
        .await;
    ice.assert_closed().await;

    // Act + Assert: route to device.
    for i in 0..BUDGET {
        route.send_candidate(format!("r{i}")).unwrap();
    }
    assert_eq!(
        route.send_candidate("r-extra".to_owned()),
        Err(CandidateError::OverBudget)
    );
    server.stop().await;
}

#[tokio::test]
async fn the_route_refuses_empty_and_oversized_candidates() {
    const TARGET: &str = "ice-route-invalid";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _, route) = webrtc_route(&server, TARGET).await;

    // Act + Assert
    for bad in [String::new(), "c".repeat(2049)] {
        assert_eq!(route.send_candidate(bad), Err(CandidateError::Invalid));
    }
    peer.assert_silent().await;
    server.stop().await;
}

/// A relay-only route has no WebRTC session: no candidates either way.
#[tokio::test]
async fn a_relay_only_route_carries_no_candidates() {
    const TARGET: &str = "ice-relay-only";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();
    let (_, res) = answered_punch(&server, &mut peer, ip, TARGET, None, WEB_RELAY, "").await;
    let mut route = res.unwrap();

    // Act + Assert
    assert_eq!(
        route.send_candidate("c".to_owned()),
        Err(CandidateError::Closed)
    );
    assert_eq!(route.next_candidate().await, None);
    server.stop().await;
}

/// Dropping the WebRoute revokes the route.
#[tokio::test]
async fn dropping_the_web_route_revokes_it() {
    const TARGET: &str = "ice-dropped";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, route) = webrtc_route(&server, TARGET).await;

    // Act
    drop(route);

    // Assert
    assert_route_gone(&server, peer.local_addr().ip(), &ctl).await;
    server.stop().await;
}

/// A route lives WEB_ROUTE_TTL (60 s): then the WebRoute yields no more candidates, refuses to
/// send, and the device's candidates are misses. Paused clock.
#[tokio::test(start_paused = true)]
async fn a_route_expires_after_its_ttl() {
    const TARGET: &str = "ice-expired";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, mut route) = webrtc_route(&server, TARGET).await;
    server.settle().await;

    // Act
    tokio::time::advance(WEB_ROUTE_TTL + Duration::from_secs(1)).await;

    // Assert
    assert_eq!(route.next_candidate().await, None);
    assert_eq!(
        route.send_candidate("late".to_owned()),
        Err(CandidateError::Closed)
    );
    assert_route_gone(&server, peer.local_addr().ip(), &ctl).await;
    server.stop().await;
}

/// Once the server stops, its routes close and new punches fail with `Stopped`.
#[tokio::test]
async fn stopping_the_server_closes_its_routes() {
    const TARGET: &str = "ice-stopped";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (_peer, _, mut route) = webrtc_route(&server, TARGET).await;
    let handle = server.handle.clone();

    // Act
    server.stop().await;

    // Assert
    let next = tokio::time::timeout(STEP, route.next_candidate()).await;
    assert_eq!(next.unwrap(), None);
    assert_eq!(
        route.send_candidate("c".to_owned()),
        Err(CandidateError::Closed)
    );
    let req = WebPunch {
        target: TARGET.to_owned(),
        offer: None,
        client: support::BROWSER_IP,
    };
    let res = handle.web_punch(req).await;
    assert_eq!(res.unwrap_err(), WebPunchError::Stopped);
}

/// The halves of a split route work independently: a receiver waiting for the device's next
/// candidate does not hold up the sender, and the route lives until both are dropped.
#[tokio::test]
async fn split_halves_work_independently_and_the_route_lives_until_both_are_gone() {
    const TARGET: &str = "ice-split";
    // Arrange
    let server = web_server(|cfg| cfg).await;
    let (mut peer, ctl, route) = webrtc_route(&server, TARGET).await;
    let ip = peer.local_addr().ip();
    let (sender, mut receiver) = route.split();
    let waiting = tokio::spawn(async move {
        let candidate = receiver.next().await;
        (candidate, receiver)
    });

    // Act + Assert: send while the receiver waits.
    sender.clone().send("to-device".to_owned()).unwrap();
    let got = expect_msg!(peer.recv().await, IceCandidate);
    assert_eq!(got.candidate, "to-device");
    let mut ice = candidate_conn(&server, ip).await;
    ice.send(&peer_candidate(&ctl, FINGERPRINT, "from-device"))
        .await;
    let (candidate, receiver) = waiting.await.unwrap();
    assert_eq!(candidate.as_deref(), Some("from-device"));

    // Act + Assert: one half alone keeps the route.
    drop(receiver);
    sender.send("still-open".to_owned()).unwrap();
    expect_msg!(peer.recv().await, IceCandidate);
    drop(sender);
    assert_route_gone(&server, ip, &ctl).await;
    server.stop().await;
}

/// A browser IP holds at most 64 punches at once: the 65th is `Busy`, other IPs are not.
#[tokio::test]
async fn a_browser_ip_at_its_route_cap_is_busy() {
    const TARGET: &str = "web-busy";
    const PER_IP: usize = 64;
    // Arrange: 64 punches waiting for the device (each route opens before its PunchHole).
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let waiting: Vec<_> = (0..PER_IP)
        .map(|_| punch(&server, TARGET, Some(offer())))
        .collect();
    for _ in 0..PER_IP {
        expect_msg!(peer.recv().await, PunchHole);
    }

    // Act
    let res = punch(&server, TARGET, None).await.unwrap();

    // Assert
    assert_eq!(res.unwrap_err(), WebPunchError::Busy);
    let other = server.handle.clone();
    let elsewhere = tokio::spawn(async move {
        let req = WebPunch {
            target: TARGET.to_owned(),
            offer: None,
            client: next_client_ip(),
        };
        other.web_punch(req).await
    });
    expect_msg!(peer.recv().await, PunchHole);
    server.stop().await;
    for task in waiting {
        assert_eq!(task.await.unwrap().unwrap_err(), WebPunchError::Stopped);
    }
    assert_eq!(
        elsewhere.await.unwrap().unwrap_err(),
        WebPunchError::Stopped
    );
}
