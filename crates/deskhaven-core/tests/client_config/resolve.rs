//! Which addresses the client configuration offers, from `public_url`, the address the
//! console was opened with, the signal port and the relay list.

use deskhaven_config::settings::HostPort;
use deskhaven_core::client_config::{ClientConfig, ConfigInput, HostSource};

use crate::{KEY, server};

fn relays(list: &[&str]) -> Vec<HostPort> {
    list.iter().map(|r| r.parse().unwrap()).collect()
}

fn resolve(
    public_url: &str,
    request_host: Option<&str>,
    signal_port: u16,
    relay_servers: &[HostPort],
) -> ClientConfig {
    ClientConfig::resolve(&ConfigInput {
        public_url,
        request_host,
        signal_port,
        relay_servers,
        public_key: KEY,
    })
}

#[test]
fn public_url_gives_host_and_api_and_the_default_port_is_omitted() {
    let cfg = resolve("https://RD.example.com/", None, 21116, &[]);

    assert_eq!(cfg.host_source, HostSource::PublicUrl);
    assert_eq!(
        cfg.server,
        server("rd.example.com", "", "https://rd.example.com", KEY)
    );
}

#[test]
fn a_custom_signal_port_is_part_of_the_id_server() {
    let cfg = resolve("https://rd.example.com", None, 21126, &[]);

    assert_eq!(cfg.server.host, "rd.example.com:21126");
}

#[test]
fn the_api_keeps_a_path_prefix_and_loses_the_trailing_slash() {
    let cfg = resolve("https://rd.example.com/deskhaven/", None, 21116, &[]);

    assert_eq!(cfg.server.api, "https://rd.example.com/deskhaven");
    assert_eq!(cfg.server.host, "rd.example.com");
}

#[test]
fn the_public_url_port_is_not_the_signal_port() {
    let cfg = resolve("http://192.0.2.7:21114", None, 21116, &[]);

    assert_eq!(cfg.server.host, "192.0.2.7");
    assert_eq!(cfg.server.api, "http://192.0.2.7:21114");
}

#[test]
fn an_ipv6_public_url_keeps_its_brackets() {
    let cfg = resolve("https://[2001:db8::1]", None, 21126, &[]);

    assert_eq!(cfg.server.host, "[2001:db8::1]:21126");
    assert_eq!(cfg.server.api, "https://[2001:db8::1]");
}

#[test]
fn without_a_public_url_the_request_host_is_used_and_flagged() {
    let cfg = resolve("", Some("Console.Example.com:8443"), 21116, &[]);

    assert_eq!(cfg.host_source, HostSource::RequestHost);
    assert_eq!(cfg.server, server("console.example.com", "", "", KEY));
}

#[test]
fn an_ipv6_request_host_is_unbracketed_then_rebracketed() {
    let cfg = resolve("", Some("[::1]:8443"), 21126, &[]);

    assert_eq!(cfg.server.host, "[::1]:21126");
}

#[test]
fn an_unusable_request_host_gives_no_host() {
    for host in [
        None,
        Some(""),
        Some("bad host"),
        Some("a/b"),
        Some("x:notaport"),
    ] {
        let cfg = resolve("", host, 21116, &[]);

        assert_eq!(cfg.host_source, HostSource::Unknown, "{host:?}");
        assert!(cfg.server.host.is_empty(), "{host:?}");
    }
}

#[test]
fn a_sole_relay_is_pinned_and_its_default_port_omitted() {
    let one = resolve(
        "https://rd.example.com",
        None,
        21116,
        &relays(&["relay.example.com:21117"]),
    );
    let other = resolve(
        "https://rd.example.com",
        None,
        21116,
        &relays(&["relay.example.com:21127"]),
    );

    assert_eq!(one.server.relay, "relay.example.com");
    assert_eq!(other.server.relay, "relay.example.com:21127");
    assert_eq!(one.relay_servers, ["relay.example.com:21117"]);
}

#[test]
fn several_relays_are_left_to_the_signal_server() {
    let cfg = resolve(
        "https://rd.example.com",
        None,
        21116,
        &relays(&["a.example.com:21117", "b.example.com:21117"]),
    );

    // A pinned `relay-server` on the controlled client wins over the server's pick
    // (07 §relay selection), which would defeat the rotation.
    assert_eq!(cfg.server.relay, "");
    assert_eq!(cfg.relay_servers.len(), 2);
}
