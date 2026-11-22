#![allow(clippy::unwrap_used)]

use super::*;
use hbb_common::AddrMangle;
use std::net::Ipv4Addr;

const FP: &str = "sha-256 97:52:D6:1F:1E:87:6C:DA:B8:21:95:64:A5:85:89:FA";
const PEER_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 5));

fn ctl(port: u16) -> SocketAddr {
    SocketAddr::from(([198, 51, 100, 7], port))
}

fn endpoint(kind: &str, sdp: &str) -> String {
    let json = serde_json::json!({ "type": kind, "sdp": sdp }).to_string();
    format!("webrtc://{}", base64::encode(json))
}

fn route(
    session_key: Option<&str>,
    expires: Instant,
) -> (PunchRoute, mpsc::Receiver<RendezvousMessage>) {
    let (tx, rx) = mpsc::channel(256);
    let route = PunchRoute {
        peer_id: "peer-1".to_owned(),
        peer_ip: PEER_IP,
        session_key: session_key.map(str::to_owned),
        to_peer: 0,
        to_ctl: 0,
        expires,
        answered: false,
        to_ctl_tx: tx,
    };
    (route, rx)
}

fn from_peer(ctl: SocketAddr, session_key: &str) -> IceCandidate {
    IceCandidate {
        socket_addr: AddrMangle::encode(ctl).into(),
        session_key: session_key.to_owned(),
        candidate: "candidate:1 1 udp 1 192.0.2.1 9 typ host".to_owned(),
        ..Default::default()
    }
}

#[test]
fn the_session_key_is_the_session_level_fingerprint() {
    let sdp = format!("v=0\r\na=fingerprint:{FP}\r\nm=application 9 x\r\na=fingerprint:other\r\n");
    assert_eq!(
        offer_session_key(&endpoint("offer", &sdp)).as_deref(),
        Some(FP)
    );
}

#[test]
fn without_one_the_first_application_fingerprint_is_the_session_key() {
    let sdp = format!(
        "v=0\nm=audio 9 x\na=fingerprint:audio\nm=application 9 x\na=fingerprint:{FP}\n\
         a=fingerprint:second\n"
    );
    assert_eq!(
        offer_session_key(&endpoint("offer", &sdp)).as_deref(),
        Some(FP)
    );
}

#[test]
fn offers_without_a_fingerprint_or_out_of_shape_have_no_session_key() {
    let long_key = format!("v=0\r\na=fingerprint:{}\r\n", "f".repeat(257));
    let fp = format!("v=0\r\na=fingerprint:{FP}\r\n");
    for offer in [
        endpoint("offer", "v=0\r\nm=application 9 x\r\n"),
        endpoint("offer", "v=0\r\nm=audio 9 x\r\na=fingerprint:audio\r\n"),
        endpoint("offer", &long_key),
        endpoint("answer", &fp),
        endpoint("offer", &fp).replacen("webrtc://", "", 1),
        format!("{}{}", endpoint("offer", &fp), " ".repeat(MAX_SDP_LEN)),
        "webrtc://%%%".to_owned(),
        format!("webrtc://{}", base64::encode("not json")),
    ] {
        assert_eq!(offer_session_key(&offer), None, "{offer}");
    }
}

#[test]
fn a_route_takes_one_answer() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    let (r, mut rx) = route(Some(FP), now + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, now));

    assert!(routes.answer(ctl(1), RendezvousMessage::new()));
    assert!(!routes.answer(ctl(1), RendezvousMessage::new()));
    assert!(!routes.answer(ctl(2), RendezvousMessage::new()));

    assert!(rx.try_recv().is_ok());
    assert!(
        rx.try_recv().is_err(),
        "the second answer was not passed on"
    );
}

#[test]
fn answers_are_accepted_only_from_the_device_while_the_route_lives() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    let (r, _rx) = route(None, now + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, now));

    assert_eq!(routes.reply(ctl(1), "peer-1", PEER_IP, now), Reply::Accept);
    let mapped = IpAddr::V6(Ipv4Addr::new(203, 0, 113, 5).to_ipv6_mapped());
    assert_eq!(routes.reply(ctl(1), "peer-1", mapped, now), Reply::Accept);
    let other = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 6));
    assert_eq!(routes.reply(ctl(1), "peer-1", other, now), Reply::Refuse);
    assert_eq!(routes.reply(ctl(1), "peer-2", PEER_IP, now), Reply::Refuse);
    let later = now + WEB_ROUTE_TTL;
    assert_eq!(
        routes.reply(ctl(1), "peer-1", PEER_IP, later),
        Reply::Refuse
    );
    assert_eq!(
        routes.reply(ctl(2), "peer-1", PEER_IP, now),
        Reply::Unrouted
    );
}

#[test]
fn device_candidates_need_the_session_the_host_and_a_secured_connection() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    let (r, mut rx) = route(Some(FP), now + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, now));
    let other = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 6));

    let to_ctl = |ice, from, trusted| routes.to_controller(ice, from, trusted, now);
    assert_eq!(to_ctl(from_peer(ctl(1), FP), PEER_IP, false), Ice::NoMatch);
    assert_eq!(to_ctl(from_peer(ctl(1), FP), other, true), Ice::NoMatch);
    assert_eq!(
        to_ctl(from_peer(ctl(1), "sha-256 00"), PEER_IP, true),
        Ice::NoMatch
    );
    assert_eq!(to_ctl(from_peer(ctl(2), FP), PEER_IP, true), Ice::NoMatch);
    assert!(rx.try_recv().is_err(), "nothing passed on");
    assert_eq!(to_ctl(from_peer(ctl(1), FP), PEER_IP, true), Ice::Forwarded);
    assert!(rx.try_recv().is_ok());
}

#[test]
fn a_route_without_a_session_carries_no_candidates() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    let (r, _rx) = route(None, now + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, now));

    let ice = from_peer(ctl(1), FP);
    assert_eq!(routes.to_controller(ice, PEER_IP, true, now), Ice::NoMatch);
    let res = routes.to_peer(ctl(1), "c".to_owned(), now);
    assert_eq!(res.unwrap_err(), CandidateError::Closed);
}

#[test]
fn each_direction_stops_at_its_budget() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    let (r, _rx) = route(Some(FP), now + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, now));

    for _ in 0..MAX_ICE_PER_DIRECTION {
        let ice = from_peer(ctl(1), FP);
        assert_eq!(
            routes.to_controller(ice, PEER_IP, true, now),
            Ice::Forwarded
        );
        let (id, msg) = routes.to_peer(ctl(1), "c".to_owned(), now).unwrap();
        assert_eq!(id, "peer-1");
        assert_eq!(msg.ice_candidate().session_key, FP);
    }
    let ice = from_peer(ctl(1), FP);
    assert_eq!(
        routes.to_controller(ice, PEER_IP, true, now),
        Ice::OverBudget
    );
    let res = routes.to_peer(ctl(1), "c".to_owned(), now);
    assert_eq!(res.unwrap_err(), CandidateError::OverBudget);
}

#[test]
fn expired_routes_close_and_are_swept() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    let (r, mut rx) = route(Some(FP), now + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, now));
    let later = now + WEB_ROUTE_TTL;

    let ice = from_peer(ctl(1), FP);
    assert_eq!(
        routes.to_controller(ice, PEER_IP, true, later),
        Ice::NoMatch
    );
    let res = routes.to_peer(ctl(1), "c".to_owned(), later);
    assert_eq!(res.unwrap_err(), CandidateError::Closed);
    routes.sweep(later);

    assert_eq!(
        routes.reply(ctl(1), "peer-1", PEER_IP, now),
        Reply::Unrouted
    );
    assert!(matches!(
        rx.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
}

#[test]
fn removing_a_route_closes_its_candidate_channel() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    let (r, mut rx) = route(Some(FP), now + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, now));

    routes.remove(ctl(1));

    assert_eq!(
        routes.reply(ctl(1), "peer-1", PEER_IP, now),
        Reply::Unrouted
    );
    assert!(matches!(
        rx.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
}

#[test]
fn routes_are_capped_per_browser_ip_and_expired_ones_make_room() {
    let now = Instant::now();
    let routes = WebRoutes::default();
    for port in 1..=MAX_ROUTES_PER_IP as u16 {
        let (r, _) = route(None, now + WEB_ROUTE_TTL);
        assert!(routes.open(ctl(port), r, now));
    }
    let (r, _) = route(None, now + WEB_ROUTE_TTL);
    assert!(!routes.open(ctl(1000), r, now), "per-IP cap");
    let (r, _) = route(None, now + WEB_ROUTE_TTL);
    let other_ip = SocketAddr::from(([198, 51, 100, 8], 1));
    assert!(routes.open(other_ip, r, now), "other IPs unaffected");

    let later = now + WEB_ROUTE_TTL;
    let (r, _) = route(None, later + WEB_ROUTE_TTL);
    assert!(
        routes.open(ctl(1000), r, later),
        "expired routes do not count"
    );
}

#[test]
fn a_connection_closes_after_too_many_misses_or_frames() {
    let routes = WebRoutes::default();
    let mut misses = IceCount::default();
    for _ in 0..MAX_ICE_MISSES {
        assert!(misses.on_candidate(&routes, from_peer(ctl(1), FP), PEER_IP, true));
    }
    assert!(!misses.on_candidate(&routes, from_peer(ctl(1), FP), PEER_IP, true));

    let (r, _rx) = route(Some(FP), Instant::now() + WEB_ROUTE_TTL);
    assert!(routes.open(ctl(1), r, Instant::now()));
    let mut frames = IceCount::default();
    for _ in 0..MAX_ICE_FRAMES {
        assert!(frames.on_candidate(&routes, from_peer(ctl(1), FP), PEER_IP, true));
    }
    assert!(!frames.on_candidate(&routes, from_peer(ctl(1), FP), PEER_IP, true));
}
