//! The `Host` header fallback is right when the operator browses to the public name, wrong
//! behind a tunnel, hence [`HostSource`]. The port of `public_url` is the HTTP port and never
//! used. A relay is never written into the string: a `relay-server` set on the client overrides
//! the signal server's pick, switching rotation and later relay changes off for every client
//! that imported it.

use deskhaven_config::settings::HostPort;
use url::{Host, Url};

use super::ServerConfig;

const DEFAULT_ID_PORT: u16 = 21116;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSource {
    PublicUrl,
    RequestHost,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigInput<'a> {
    pub public_url: &'a str,
    pub request_host: Option<&'a str>,
    pub signal_port: u16,
    pub relay_servers: &'a [HostPort],
    pub public_key: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    pub server: ServerConfig,
    pub host_source: HostSource,
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

fn with_port(host: &str, port: u16, default: u16) -> String {
    if port == default {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    }
}

fn api_base(url: &Url) -> String {
    let Some(host) = url.host_str() else {
        return String::new();
    };
    let port = url.port().map_or_else(String::new, |p| format!(":{p}"));
    let path = url.path().trim_end_matches('/');
    format!("{}://{host}{port}{path}", url.scheme())
}

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

fn is_local(host: &Host<&str>) -> bool {
    match host {
        Host::Domain(name) => *name == "localhost" || name.ends_with(".localhost"),
        Host::Ipv4(ip) => ip.is_loopback() || ip.is_unspecified(),
        Host::Ipv6(ip) => ip.is_loopback() || ip.is_unspecified(),
    }
}
