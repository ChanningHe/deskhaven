//! Real client IP resolution (06-security §3.6).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::{IpAddr, SocketAddr};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::routing::get;
use deskhaven_http::{ClientIp, HttpConfig, TrustedProxies, X_FORWARDED_FOR, apply_middleware};
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
    let mut headers = HeaderMap::new();
    for v in values {
        headers.append(X_FORWARDED_FOR, HeaderValue::from_str(v).unwrap());
    }
    headers
}

#[test]
fn no_trusted_proxies_ignores_xff() {
    let resolved = TrustedProxies::default().resolve(ip("10.0.0.1"), &xff(&["1.2.3.4"]));
    assert_eq!(resolved, ip("10.0.0.1"));
}

#[test]
fn untrusted_peer_ignores_xff() {
    let resolved = trusted(&["10.0.0.0/8"]).resolve(ip("203.0.113.9"), &xff(&["1.2.3.4"]));
    assert_eq!(resolved, ip("203.0.113.9"));
}

#[test]
fn trusted_peer_without_xff_is_the_client() {
    let resolved = trusted(&["10.0.0.0/8"]).resolve(ip("10.0.0.1"), &HeaderMap::new());
    assert_eq!(resolved, ip("10.0.0.1"));
}

#[test]
fn single_trusted_hop_uses_rightmost_entry() {
    let resolved = trusted(&["10.0.0.1"]).resolve(ip("10.0.0.1"), &xff(&["198.51.100.7"]));
    assert_eq!(resolved, ip("198.51.100.7"));
}

#[test]
fn multi_hop_trusted_chain_skips_all_trusted_hops() {
    let proxies = trusted(&["10.0.0.0/8", "192.168.0.0/16"]);
    let headers = xff(&["198.51.100.7, 192.168.1.1, 10.1.1.1"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        ip("198.51.100.7")
    );
}

#[test]
fn spoofed_leftmost_entries_are_ignored() {
    let proxies = trusted(&["10.0.0.0/8"]);
    // The client sent "XFF: 1.1.1.1, 10.9.9.9"; the proxy appended the real peer.
    let headers = xff(&["1.1.1.1, 10.9.9.9, 203.0.113.50"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        ip("203.0.113.50")
    );
}

#[test]
fn multiple_xff_headers_are_one_list_in_order() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["6.6.6.6", "198.51.100.7, 10.0.0.2"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        ip("198.51.100.7")
    );
}

#[test]
fn chain_of_only_trusted_hops_yields_leftmost() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["10.0.0.3, 10.0.0.2"]);
    assert_eq!(proxies.resolve(ip("10.0.0.1"), &headers), ip("10.0.0.3"));
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
            ip("10.0.0.5"),
            "entry {bad:?}"
        );
    }
}

#[test]
fn garbage_rightmost_entry_yields_peer() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["198.51.100.7, nonsense"]);
    assert_eq!(proxies.resolve(ip("10.0.0.1"), &headers), ip("10.0.0.1"));
}

#[test]
fn garbage_left_of_untrusted_entry_is_never_reached() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let headers = xff(&["nonsense,,198.51.100.7"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &headers),
        ip("198.51.100.7")
    );
}

#[test]
fn non_ascii_header_value_is_garbage() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let mut headers = HeaderMap::new();
    headers.append(
        X_FORWARDED_FOR,
        HeaderValue::from_bytes(b"1.2.3.4\xff").unwrap(),
    );
    assert_eq!(proxies.resolve(ip("10.0.0.1"), &headers), ip("10.0.0.1"));
}

#[test]
fn ipv6_chain_resolves() {
    let proxies = trusted(&["fd00::/8"]);
    let headers = xff(&["2001:db8::1, fd00::2"]);
    assert_eq!(proxies.resolve(ip("fd00::1"), &headers), ip("2001:db8::1"));
}

#[test]
fn ipv4_mapped_peer_matches_ipv4_cidr_and_is_reported_as_ipv4() {
    let proxies = trusted(&["10.0.0.0/8"]);
    let resolved = proxies.resolve(ip("::ffff:10.0.0.1"), &xff(&["::ffff:198.51.100.7"]));
    assert_eq!(resolved, ip("198.51.100.7"));
    let untrusted = TrustedProxies::default().resolve(ip("::ffff:203.0.113.1"), &HeaderMap::new());
    assert_eq!(untrusted, ip("203.0.113.1"));
}

#[test]
fn ipv4_mapped_cidr_matches_ipv4_peer() {
    let proxies = trusted(&["::ffff:10.0.0.0/104"]);
    assert_eq!(
        proxies.resolve(ip("10.0.0.1"), &xff(&["198.51.100.7"])),
        ip("198.51.100.7")
    );
}

async fn client_ip_body(app: Router, peer: &str, xff_value: Option<&str>) -> (StatusCode, String) {
    let app = app.layer(MockConnectInfo(peer.parse::<SocketAddr>().unwrap()));
    let mut req = Request::get("/ip");
    if let Some(v) = xff_value {
        req = req.header(X_FORWARDED_FOR, v);
    }
    let res = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = res.status();
    let body = to_bytes(res.into_body(), 1024).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

fn ip_router() -> Router {
    Router::new().route(
        "/ip",
        get(|ClientIp(ip): ClientIp| async move { ip.to_string() }),
    )
}

#[tokio::test]
async fn extractor_uses_trusted_proxies_from_middleware() {
    let cfg = HttpConfig {
        trusted_proxies: trusted(&["10.0.0.0/8"]),
        ..HttpConfig::default()
    };
    let app = apply_middleware(ip_router(), &cfg);
    let got = client_ip_body(app, "10.0.0.1:4000", Some("198.51.100.7")).await;
    assert_eq!(got, (StatusCode::OK, "198.51.100.7".to_owned()));
}

#[tokio::test]
async fn extractor_defaults_to_peer() {
    let app = apply_middleware(ip_router(), &HttpConfig::default());
    let got = client_ip_body(app, "[::ffff:203.0.113.1]:4000", Some("1.2.3.4")).await;
    assert_eq!(got, (StatusCode::OK, "203.0.113.1".to_owned()));
}

#[tokio::test]
async fn extractor_without_trusted_proxies_extension_is_500() {
    let (status, body) = client_ip_body(ip_router(), "10.0.0.1:4000", None).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body, "internal server error");
}

#[tokio::test]
async fn extractor_without_connect_info_is_500() {
    let app = apply_middleware(ip_router(), &HttpConfig::default());
    let res = app
        .oneshot(Request::get("/ip").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

fn arb_ip() -> impl Strategy<Value = IpAddr> {
    prop_oneof![
        any::<[u8; 4]>().prop_map(IpAddr::from),
        any::<[u8; 16]>().prop_map(IpAddr::from),
    ]
}

proptest! {
    #[test]
    fn prop_no_trusted_proxies_always_yields_canonical_peer(
        peer in arb_ip(),
        header in "[ -~]{0,64}",
    ) {
        let headers = xff(&[&header]);
        prop_assert_eq!(TrustedProxies::default().resolve(peer, &headers), peer.to_canonical());
    }

    #[test]
    fn prop_untrusted_rightmost_entry_wins_regardless_of_prefix(
        spoofed in proptest::collection::vec("[ -+\\--~]{0,20}", 0..5),
        client in any::<[u8; 4]>().prop_filter("outside 10/8", |o| o[0] != 10),
        hops in proptest::collection::vec(any::<[u8; 3]>(), 0..4),
    ) {
        let proxies = trusted(&["10.0.0.0/8"]);
        let client = IpAddr::from(client);
        let mut entries = spoofed;
        entries.push(client.to_string());
        entries.extend(hops.iter().map(|h| format!("10.{}.{}.{}", h[0], h[1], h[2])));
        let headers = xff(&[&entries.join(",")]);
        prop_assert_eq!(proxies.resolve(ip("10.0.0.1"), &headers), client);
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
        prop_assert!(allowed.contains(&got));
    }
}
