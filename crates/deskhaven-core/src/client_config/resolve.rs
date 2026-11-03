//! The addresses to offer, from what the deployment knows.
//!
//! - ID server: the host of `public_url`, else the host the console was opened with (the
//!   `Host` header; right when the operator browses to the public name, wrong behind a
//!   tunnel, hence [`HostSource`]), plus the signal port when it is not the client's
//!   default 21116. The port of `public_url` is the HTTP port and never used here.
//! - API server: `public_url` without a trailing slash (the client strips one itself,
//!   `common.dart:3589-3593`), empty when unset.
//! - Relay server: only a *single* configured relay. A `relay-server` set on the controlled
//!   client overrides the signal server's pick (07-hbbs-fork), so pinning one of several would
//!   switch the rotation off; with none or several the signal server hands one out.

use deskhaven_config::settings::HostPort;
use url::Url;

use super::ServerConfig;

/// The ports the client assumes for an address without one (hbbs, hbbr).
const DEFAULT_ID_PORT: u16 = 21116;
const DEFAULT_RELAY_PORT: u16 = 21117;

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
                relay: pinned_relay(input.relay_servers),
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

fn pinned_relay(relays: &[HostPort]) -> String {
    match relays {
        [relay] => {
            let host = relay.host();
            let host = if host.contains(':') {
                format!("[{host}]")
            } else {
                host.to_owned()
            };
            with_port(&host, relay.port(), DEFAULT_RELAY_PORT)
        }
        _ => String::new(),
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
    if is_authority {
        url.host_str().map(str::to_owned)
    } else {
        None
    }
}
