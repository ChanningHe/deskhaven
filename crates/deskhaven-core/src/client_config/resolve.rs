//! The addresses to offer, from what the deployment knows.
//!
//! - ID server: the host of `public_url`, else the host the console was opened with (the
//!   `Host` header; right when the operator browses to the public name, wrong behind a
//!   tunnel, hence [`HostSource`]), plus the signal port when it is not the client's
//!   default 21116. The port of `public_url` is the HTTP port and never used here.
//! - API server: `public_url` without a trailing slash (the client strips one itself,
//!   `common.dart:3589-3593`), empty when unset.
//! - Relay server: never written into the string. A `relay-server` set on the controlled
//!   client overrides the signal server's pick (07-hbbs-fork), which would switch rotation
//!   and later relay changes off for every client that imported it. The configured relays
//!   are only listed for information.
//!
//! A `Host` header naming this machine (`localhost`, loopback, unspecified) is no address a
//! client elsewhere can use, so it counts as no host at all.

use deskhaven_config::settings::HostPort;
use url::{Host, Url};

use super::ServerConfig;

/// The port the client assumes for an ID server address without one (hbbs).
const DEFAULT_ID_PORT: u16 = 21116;

/// Where the ID server host came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSource {
    /// `public_url`: the address operators chose for clients.
    PublicUrl,
    /// The address the console was opened with; may not be reachable from clients.
    RequestHost,
    /// Neither is usable.
    Unknown,
}

/// Everything [`ClientConfig::resolve`] reads.
#[derive(Debug, Clone, Copy)]
pub struct ConfigInput<'a> {
    /// `public_url` as configured; blank = unset.
    pub public_url: &'a str,
    /// The `Host` header of the console request, if any.
    pub request_host: Option<&'a str>,
    /// `signal.port`.
    pub signal_port: u16,
    /// `signal.relay_servers`.
    pub relay_servers: &'a [HostPort],
    /// The hbbs public key (base64).
    pub public_key: &'a str,
}

/// The offered configuration and how it was derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub server: ServerConfig,
    pub host_source: HostSource,
    /// Every configured relay as `host:port` (the config string pins at most one).
    pub relay_servers: Vec<String>,
}

impl ClientConfig {
    pub fn resolve(input: &ConfigInput<'_>) -> Self {
        let public = Url::parse(input.public_url)
            .ok()
            .filter(|url| url.host_str().is_some());
        let from_url = public.as_ref().and_then(|url| url.host_str());
        let (host, host_source) = match (from_url, request_host(input.request_host)) {
            (Some(host), _) => (host.to_owned(), HostSource::PublicUrl),
            (None, Some(host)) => (host, HostSource::RequestHost),
            (None, None) => (String::new(), HostSource::Unknown),
        };
        let host = if host.is_empty() {
            host
        } else {
            with_port(&host, input.signal_port, DEFAULT_ID_PORT)
        };
        Self {
            server: ServerConfig {
                host,
                relay: String::new(),
                api: public.as_ref().map_or_else(String::new, api_base),
                key: input.public_key.to_owned(),
            },
            host_source,
            relay_servers: input
                .relay_servers
                .iter()
                .map(ToString::to_string)
                .collect(),
        }
    }
}

/// `host[:port]` with the port left out when it is the client's default.
fn with_port(host: &str, port: u16, default: u16) -> String {
    if port == default {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    }
}

/// `scheme://host[:port][/path]`, no trailing slash.
fn api_base(url: &Url) -> String {
    let Some(host) = url.host_str() else {
        return String::new();
    };
    let port = url.port().map_or_else(String::new, |p| format!(":{p}"));
    let path = url.path().trim_end_matches('/');
    format!("{}://{host}{port}{path}", url.scheme())
}

/// The host of a `Host` header (`name`, `name:port`, `[v6]:port`), lowercase, IPv6 in
/// brackets; `None` for anything that is not just an authority.
fn request_host(raw: Option<&str>) -> Option<String> {
    let url = Url::parse(&format!("http://{}/", raw?)).ok()?;
    let is_authority = url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    if !is_authority || url.host().is_none_or(|host| is_local(&host)) {
        return None;
    }
    url.host_str().map(str::to_owned)
}

/// This machine, as seen from itself: `localhost`, loopback and unspecified addresses.
fn is_local(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => *name == "localhost" || name.ends_with(".localhost"),
        Host::Ipv4(ip) => ip.is_loopback() || ip.is_unspecified(),
        Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}
