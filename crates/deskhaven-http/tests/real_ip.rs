//! Real client IP resolution (06-security §3.6).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, SocketAddr};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::routing::get;
use deskhaven_http::{
    ClientIp, ErrorBodies, HttpConfig, InternalPeer, TrustedProxies, X_FORWARDED_FOR,
    apply_middleware,
};
use ipnet::IpNet;
use proptest::prelude::*;
use tower::ServiceExt;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

fn trusted(cidrs: &[&str]) -> TrustedProxies {
    TrustedProxies::new(cidrs.iter().map(|c| {
        c.parse::<IpNet>()
            .unwrap_or_else(|_| IpNet::from(c.parse::<IpAddr>().unwrap()))
    }))
}

fn xff(values: &[&str]) -> HeaderMap {
    xff_bytes(&values.iter().map(|v| v.as_bytes()).collect::<Vec<_>>())
}

fn xff_bytes(values: &[&[u8]]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for v in values {
        headers.append(X_FORWARDED_FOR, HeaderValue::from_bytes(v).unwrap());
    }
    headers
}

fn client(addr: &str) -> ClientIp {
    ClientIp {
        addr: ip(addr),
        is_proxy_fallback: false,
    }
}

fn fallback(addr: &str) -> ClientIp {
    ClientIp {
        addr: ip(addr),
        is_proxy_fallback: true,
    }
}

#[test]
fn no_trusted_proxies_ignores_xff() {
    let resolved = TrustedProxies::default().resolve(ip("10.0.0.1"), &xff(&["1.2.3.4"]));
    assert_eq!(resolved, client("10.0.0.1"));
}

#[test]
fn untrusted_peer_ignores_xff() {
    let resolved = trusted(&["10.0.0.0/8"]).resolve(ip("203.0.113.9"), &xff(&["1.2.3.4"]));
    assert_eq!(resolved, client("203.0.113.9"));
}

#[test]
fn trusted_peer_without_xff_is_a_proxy_fallback() {
    let resolved = trusted(&["10.0.0.0/8"]).resolve(ip("10.0.0.1"), &HeaderMap::new());
    assert_eq!(resolved, fallback("10.0.0.1"));
}

#[test]
fn single_trusted_hop_uses_rightmost_entry() {
    let resolved = trusted(&["10.0.0.1"]).resolve(ip("10.0.0.1"), &xff(&["198.51.100.7"]));
    assert_eq!(resolved, client("198.51.100.7"));
}

#[test]
fn multi_hop_trusted_chain_skips_all_trusted_hops() {
    let proxies = trusted(&["10.0.0.0/8", "192.168.0.0/16"]);
    let headers = xff(&["198.51.100.7, 192.168.1.1, 10.1.1.1"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        client("198.51.100.7")
    );
}

#[test]
fn spoofed_leftmost_entries_are_ignored() {
    let proxies = trusted(&["10.0.0.0/8"]);
    // The client sent "XFF: 1.1.1.1, 10.9.9.9"; the proxy appended the real peer.
    let headers = xff(&["1.1.1.1, 10.9.9.9, 203.0.113.50"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        client("203.0.113.50")
    );
}

#[test]
fn multiple_xff_headers_are_one_list_in_order() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["6.6.6.6", "198.51.100.7, 10.0.0.2"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        client("198.51.100.7")
    );
}

#[test]
fn chain_of_only_trusted_hops_yields_leftmost_as_fallback() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["10.0.0.3, 10.0.0.2"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        fallback("10.0.0.3")
    );
}

#[test]
fn garbage_entry_stops_at_last_validated_address() {
    let proxies = trusted(&["10.0.0.0/8"]);
    for bad in [
        "garbage",
        "",
        " ",
        "1.2.3.4:80",
        "[::1]",
        "unknown",
        "1.2.3",
    ] {
        let headers = xff(&[&format!("{bad}, 10.0.0.5")]);
        assert_eq!(
            proxies.resolve(ip("10.0.0.1"), &headers),
            fallback("10.0.0.5"),
            "entry {bad:?}"
        );
    }
}

#[test]
fn garbage_rightmost_entry_yields_peer_as_fallback() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["198.51.100.7, nonsense"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        fallback("10.0.0.1")
    );
}

#[test]
fn garbage_left_of_untrusted_entry_is_never_reached() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["nonsense,,198.51.100.7"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        client("198.51.100.7")
    );
}

/// Review H1: nginx `$proxy_add_x_forwarded_for` turns a client-sent
/// `X-Forwarded-For: \xff` into `"\xff, <real ip>"`. The obs-text byte sits in
/// an entry the walk never reaches, so it must not affect the result.
#[test]
fn obs_text_left_of_untrusted_entry_is_never_reached() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff_bytes(&[b"\xff, 203.0.113.50"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        client("203.0.113.50")
    );
}

#[test]
fn obs_text_in_an_earlier_header_is_never_reached() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff_bytes(&[b"\xfe\xff", b"203.0.113.50"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        client("203.0.113.50")
    );
}

#[test]
fn obs_text_in_a_reached_entry_falls_back_to_the_trusted_hop() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff_bytes(&[b"198.51.100.7, 1.2.3.4\xff"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        fallback("10.0.0.1")
    );
}

#[test]
fn ipv6_chain_resolves() {
    let proxies = trusted(&["fd00::/8"]);
    let headers = xff(&["2001:db8::1, fd00::2"]);
    assert_eq!(
        proxies.resolve(ip("fd00::1"), &headers),
        client("2001:db8::1")
    );
}

#[test]
fn ipv4_mapped_peer_matches_ipv4_cidr_and_is_reported_as_ipv4() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let resolved = proxies.resolve(ip("::ffff:10.0.0.1"), &xff(&["::ffff:198.51.100.7"]));
    assert_eq!(resolved, client("198.51.100.7"));
    let untrusted = TrustedProxies::default().resolve(ip("::ffff:203.0.113.1"), &HeaderMap::new());
    assert_eq!(untrusted, client("203.0.113.1"));
}

#[test]
fn ipv4_mapped_cidr_matches_ipv4_peer() {
    let proxies = trusted(&["::ffff:10.0.0.0/104"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &xff(&["198.51.100.7"])),
        client("198.51.100.7")
    );
}

#[test]
fn ipv6_any_does_not_trust_ipv4_or_mapped_peers() {
    let proxies = trusted(&["::/0"]);
    assert!(!proxies.is_trusted(ip("10.0.0.1")));
    assert!(!proxies.is_trusted(ip("::ffff:10.0.0.1")));
    assert_eq!(
        proxies.resolve(ip("::ffff:10.0.0.1"), &xff(&["1.2.3.4"])),
        client("10.0.0.1")
    );
}

// --- extractor ---------------------------------------------------------------

fn ip_router() -> Router {
    Router::new().route(
        "/ip",
        get(|ip: ClientIp| async move { format!("{} {}", ip.addr, ip.is_proxy_fallback) }),
    )
}

fn with_proxies(cidrs: &[&str]) -> Router {
    let cfg = HttpConfig {
        trusted_proxies: trusted(cidrs),
        ..HttpConfig::default()
    };
    apply_middleware(ip_router(), &cfg, ErrorBodies::default())
}

async fn call(app: Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.oneshot(req).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), 1024).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn ip_request(xff_value: Option<&str>) -> Request<Body> {
    let mut req = Request::get("/ip");
    if let Some(v) = xff_value {
        req = req.header(X_FORWARDED_FOR, v);
    }
    req.body(Body::empty()).unwrap()
}

fn peer(app: Router, addr: &str) -> Router {
    app.layer(MockConnectInfo(addr.parse::<SocketAddr>().unwrap()))
}

#[tokio::test]
async fn extractor_uses_trusted_proxies_from_middleware() {
    let app = peer(with_proxies(&["10.0.0.0/8"]), "10.0.0.1:4000");
    let got = call(app, ip_request(Some("198.51.100.7"))).await;
    assert_eq!(got, (StatusCode::OK, "198.51.100.7 false".to_owned()));
}

#[tokio::test]
async fn extractor_defaults_to_peer() {
    let app = peer(with_proxies(&[]), "[::ffff:203.0.113.1]:4000");
    let got = call(app, ip_request(Some("1.2.3.4"))).await;
    assert_eq!(got, (StatusCode::OK, "203.0.113.1 false".to_owned()));
}

#[tokio::test]
async fn extractor_reports_proxy_fallback() {
    let app = peer(with_proxies(&["10.0.0.0/8"]), "10.0.0.1:4000");
    let got = call(app, ip_request(Some("garbage"))).await;
    assert_eq!(got, (StatusCode::OK, "10.0.0.1 true".to_owned()));
}

#[tokio::test]
async fn internal_peer_takes_precedence_and_ignores_xff() {
    let app = peer(with_proxies(&["10.0.0.0/8"]), "10.0.0.1:4000");
    let mut req = ip_request(Some("198.51.100.7"));
    req.extensions_mut()
        .insert(InternalPeer("203.0.113.77:5000".parse().unwrap()));
    let got = call(app, req).await;
    assert_eq!(got, (StatusCode::OK, "203.0.113.77 false".to_owned()));
}

#[tokio::test]
async fn internal_peer_works_without_connect_info_and_is_never_a_proxy_fallback() {
    let app = with_proxies(&["10.0.0.0/8"]);
    let mut req = ip_request(Some("198.51.100.7"));
    req.extensions_mut()
        .insert(InternalPeer("10.0.0.9:5000".parse().unwrap()));
    let got = call(app, req).await;
    assert_eq!(got, (StatusCode::OK, "10.0.0.9 false".to_owned()));
}

#[tokio::test]
async fn extractor_without_trusted_proxies_extension_is_500() {
    let app = peer(ip_router(), "10.0.0.1:4000");
    let (status, body) = call(app, ip_request(None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, "internal server error");
}

#[tokio::test]
async fn extractor_without_peer_is_500() {
    let (status, _) = call(with_proxies(&[]), ip_request(None)).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
}

// --- properties --------------------------------------------------------------

fn arb_ip() -> impl Strategy<Value = IpAddr> {
    prop_oneof![
        any::<[u8; 4]>().prop_map(IpAddr::from),
        any::<[u8; 16]>().prop_map(IpAddr::from),
    ]
}

/// Bytes a `HeaderValue` may hold, minus the comma: visible ASCII, space, tab, obs-text.
fn arb_entry_bytes() -> impl Strategy<Value = Vec<u8>> {
    let byte = prop_oneof![Just(b'\t'), 0x20u8..=0x2B, 0x2Du8..=0x7E, 0x80u8..=0xFF];
    proptest::collection::vec(byte, 0..20)
}

/// Anything that does not parse as an address.
fn arb_garbage() -> impl Strategy<Value = Vec<u8>> {
    arb_entry_bytes().prop_filter("not an ip", |b| {
        std::str::from_utf8(b).map_or(true, |s| s.trim().parse::<IpAddr>().is_err())
    })
}

fn join(entries: &[Vec<u8>]) -> Vec<u8> {
    entries.join(&b","[..])
}

fn ten_net(hops: &[[u8; 3]]) -> Vec<Vec<u8>> {
    hops.iter()
        .map(|h| format!("10.{}.{}.{}", h[0], h[1], h[2]).into_bytes())
        .collect()
}

proptest! {
    #[test]
    fn prop_no_trusted_proxies_always_yields_canonical_peer(
        peer in arb_ip(),
        header in arb_entry_bytes(),
    ) {
        let headers = xff_bytes(&[&header]);
        let got = TrustedProxies::default().resolve(peer, &headers);
        prop_assert_eq!(got.addr, peer.to_canonical());
        prop_assert!(!got.is_proxy_fallback);
    }

    #[test]
    fn prop_untrusted_rightmost_entry_wins_regardless_of_prefix(
        spoofed in proptest::collection::vec(arb_entry_bytes(), 0..5),
        client in any::<[u8; 4]>().prop_filter("outside 10/8", |o| o[0] != 10),
        hops in proptest::collection::vec(any::<[u8; 3]>(), 0..4),
        split_headers in any::<bool>(),
    ) {
        let proxies = trusted(&["10.0.0.0/8"]);
        let client = IpAddr::from(client);
        let mut tail = vec![client.to_string().into_bytes()];
        tail.extend(ten_net(&hops));
        let headers = if split_headers {
            xff_bytes(&[&join(&spoofed), &join(&tail)])
        } else {
            let mut all = spoofed;
            all.extend(tail);
            xff_bytes(&[&join(&all)])
        };
        let got = proxies.resolve(ip("10.0.0.1"), &headers);
        prop_assert_eq!(got, ClientIp { addr: client, is_proxy_fallback: false });
    }

    #[test]
    fn prop_garbage_in_trusted_zone_falls_back_to_leftmost_validated_hop(
        prefix in proptest::collection::vec(arb_entry_bytes(), 0..3),
        garbage in arb_garbage(),
        hops in proptest::collection::vec(any::<[u8; 3]>(), 0..4),
    ) {
        let proxies = trusted(&["10.0.0.0/8"]);
        let peer = ip("10.0.0.1");
        let hop_entries = ten_net(&hops);
        let expected = hop_entries
            .first()
            .map_or(peer, |h| ip(std::str::from_utf8(h).unwrap()));
        let mut all = prefix;
        all.push(garbage);
        all.extend(hop_entries);
        let got = proxies.resolve(peer, &xff_bytes(&[&join(&all)]));
        prop_assert_eq!(got, ClientIp { addr: expected, is_proxy_fallback: true });
    }

    #[test]
    fn prop_result_is_peer_or_listed_entry(
        peer in arb_ip(),
        entries in proptest::collection::vec(arb_ip(), 0..6),
    ) {
        let proxies = trusted(&["0.0.0.0/1", "::/1"]);
        let list: Vec<String> = entries.iter().map(ToString::to_string).collect();
        let headers = xff(&[&list.join(", ")]);
        let got = proxies.resolve(peer, &headers);
        let allowed: Vec<IpAddr> = std::iter::once(peer)
            .chain(entries)
            .map(|a| a.to_canonical())
            .collect();
        prop_assert!(allowed.contains(&got.addr));
        prop_assert_eq!(got.is_proxy_fallback, proxies.is_trusted(got.addr));
    }
}
