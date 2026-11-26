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

async fn assert_no_candidate(route: &mut WebRoute) {
    if let Ok(c) = tokio::time::timeout(QUIET, route.next_candidate()).await {
        panic!("unexpected candidate {c:?}");
    }
}

async fn send_misses(conn: &mut TcpClient, ctl: &Bytes, n: usize) {
    for i in 0..n {
        conn.send(&peer_candidate(ctl, "sha-256 00:00", &format!("miss-{i}")))
            .await;
    }
}

#[tokio::test]
async fn candidates_flow_both_ways_for_a_udp_registered_device() {
    const TARGET: &str = "ice-udp-device";
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();
    let task = punch(&server, TARGET, Some(offer()));
    let ph = expect_msg!(peer.recv().await, PunchHole);
    let mut ice = candidate_conn(&server, ip).await;

    ice.send(&peer_candidate(&ph.socket_addr, FINGERPRINT, "early"))
        .await;
    send_relay_response(&server, ip, relay_answer(&ph, TARGET, WEB_RELAY, ANSWER)).await;
    let mut route = task.await.unwrap().unwrap();
    ice.send(&peer_candidate(&ph.socket_addr, FINGERPRINT, "late"))
        .await;
    route.send_candidate("to-device".to_owned()).unwrap();

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

#[tokio::test]
async fn candidates_reach_a_tcp_registered_device_on_its_connection() {
    const TARGET: &str = "ice-tcp-device";
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

    route.send_candidate("to-device".to_owned()).unwrap();

    let got = expect_msg!(peer.recv().await, IceCandidate);
    assert_eq!(
        (got.session_key.as_str(), got.candidate.as_str()),
        (FINGERPRINT, "to-device")
    );
    server.stop().await;
}

#[tokio::test]
async fn forged_candidates_are_dropped() {
    const TARGET: &str = "ice-forged";
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, mut route) = webrtc_route(&server, TARGET).await;
    let ip = peer.local_addr().ip();
    let elsewhere = encode_addr("198.51.100.200:9".parse().unwrap());
    let mut plain = TcpClient::connect_from(ip, server.main_addr()).await;

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

    assert_no_candidate(&mut route).await;
    ice.send(&peer_candidate(&ctl, FINGERPRINT, "real")).await;
    assert_eq!(route.next_candidate().await.as_deref(), Some("real"));
    server.stop().await;
}

#[tokio::test]
async fn oversized_and_unmatched_candidates_are_misses_that_close_the_connection() {
    const TARGET: &str = "ice-misses";
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

    ice.send(&peer_candidate(&ctl, FINGERPRINT, &long_candidate))
        .await;
    ice.send(&peer_candidate(&ctl, &long_key, "long-key")).await;
    ice.send(&wire_controller).await;
    send_misses(&mut ice, &ctl, support::MAX_ICE_MISSES - 3).await;
    ice.send(&peer_candidate(&ctl, FINGERPRINT, &"c".repeat(2048)))
        .await;

    assert_eq!(route.next_candidate().await.map(|c| c.len()), Some(2048));
    send_misses(&mut ice, &ctl, 1).await;
    ice.assert_closed().await;
    server.stop().await;
}

#[tokio::test]
async fn each_direction_has_a_budget_and_a_connection_a_frame_cap() {
    const TARGET: &str = "ice-budget";
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, mut route) = webrtc_route(&server, TARGET).await;
    let mut ice = candidate_conn(&server, peer.local_addr().ip()).await;

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
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _, route) = webrtc_route(&server, TARGET).await;

    for bad in [String::new(), "c".repeat(2049)] {
        assert_eq!(route.send_candidate(bad), Err(CandidateError::Invalid));
    }
    peer.assert_silent().await;
    server.stop().await;
}

#[tokio::test]
async fn a_relay_only_route_carries_no_candidates() {
    const TARGET: &str = "ice-relay-only";
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let ip = peer.local_addr().ip();
    let (_, res) = answered_punch(&server, &mut peer, ip, TARGET, None, WEB_RELAY, "").await;
    let mut route = res.unwrap();

    assert_eq!(
        route.send_candidate("c".to_owned()),
        Err(CandidateError::Closed)
    );
    assert_eq!(route.next_candidate().await, None);
    server.stop().await;
}

#[tokio::test]
async fn dropping_the_web_route_revokes_it() {
    const TARGET: &str = "ice-dropped";
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, route) = webrtc_route(&server, TARGET).await;

    drop(route);

    assert_route_gone(&server, peer.local_addr().ip(), &ctl).await;
    server.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_route_expires_after_its_ttl() {
    const TARGET: &str = "ice-expired";
    let server = web_server(|cfg| cfg).await;
    let (peer, ctl, mut route) = webrtc_route(&server, TARGET).await;
    server.settle().await;

    tokio::time::advance(WEB_ROUTE_TTL + Duration::from_secs(1)).await;

    assert_eq!(route.next_candidate().await, None);
    assert_eq!(
        route.send_candidate("late".to_owned()),
        Err(CandidateError::Closed)
    );
    assert_route_gone(&server, peer.local_addr().ip(), &ctl).await;
    server.stop().await;
}

#[tokio::test]
async fn stopping_the_server_closes_its_routes() {
    const TARGET: &str = "ice-stopped";
    let server = web_server(|cfg| cfg).await;
    let (_peer, _, mut route) = webrtc_route(&server, TARGET).await;
    let handle = server.handle.clone();

    server.stop().await;

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

#[tokio::test]
async fn split_halves_work_independently_and_the_route_lives_until_both_are_gone() {
    const TARGET: &str = "ice-split";
    let server = web_server(|cfg| cfg).await;
    let (mut peer, ctl, route) = webrtc_route(&server, TARGET).await;
    let ip = peer.local_addr().ip();
    let (sender, mut receiver) = route.split();
    let waiting = tokio::spawn(async move {
        let candidate = receiver.next().await;
        (candidate, receiver)
    });

    sender.clone().send("to-device".to_owned()).unwrap();
    let got = expect_msg!(peer.recv().await, IceCandidate);
    assert_eq!(got.candidate, "to-device");
    let mut ice = candidate_conn(&server, ip).await;
    ice.send(&peer_candidate(&ctl, FINGERPRINT, "from-device"))
        .await;
    let (candidate, receiver) = waiting.await.unwrap();
    assert_eq!(candidate.as_deref(), Some("from-device"));

    drop(receiver);
    sender.send("still-open".to_owned()).unwrap();
    expect_msg!(peer.recv().await, IceCandidate);
    drop(sender);
    assert_route_gone(&server, ip, &ctl).await;
    server.stop().await;
}

#[tokio::test]
async fn a_browser_ip_at_its_route_cap_is_busy() {
    const TARGET: &str = "web-busy";
    const PER_IP: usize = 64;
    // 64 punches waiting for the device (each route opens before its PunchHole).
    let server = web_server(|cfg| cfg).await;
    let (mut peer, _) = server.online_peer(TARGET).await;
    let waiting: Vec<_> = (0..PER_IP)
        .map(|_| punch(&server, TARGET, Some(offer())))
        .collect();
    for _ in 0..PER_IP {
        expect_msg!(peer.recv().await, PunchHole);
    }

    let res = punch(&server, TARGET, None).await.unwrap();

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
