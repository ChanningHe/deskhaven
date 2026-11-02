//! Type-erased setting values and the `host:port` list item.

use std::fmt;
use std::net::Ipv6Addr;
use std::str::FromStr;

use serde_json::{Value as Json, json};

use crate::{Duration, Secret};

/// An effective setting value, already validated against its declaration.
/// `Debug` never reveals secrets.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Value {
    Bool(bool),
    U32(u32),
    Duration(Duration),
    /// One of the declared variants (always a `'static` variant name).
    Enum(&'static str),
    HostPorts(Vec<HostPort>),
    /// Plain text; empty means "not configured".
    Text(String),
    /// Sensitive text; empty means "not configured".
    Secret(Secret),
}

impl Value {
    /// Public JSON form, as stored for plain values and shown by read APIs.
    /// Secrets render as `{"set": bool}`; use [`Value::secret`] to seal them.
    pub fn to_json(&self) -> Json {
        match self {
            Self::Bool(b) => Json::Bool(*b),
            Self::U32(n) => Json::from(*n),
            Self::Duration(d) => Json::String(d.to_string()),
            Self::Enum(variant) => Json::String((*variant).to_owned()),
            Self::HostPorts(list) => list.iter().map(|h| Json::String(h.to_string())).collect(),
            Self::Text(text) => Json::String(text.clone()),
            Self::Secret(secret) => json!({ "set": !secret.is_empty() }),
        }
    }

    /// The secret, for the one caller that must encrypt it at rest.
    pub fn secret(&self) -> Option<&Secret> {
        match self {
            Self::Secret(secret) => Some(secret),
            _ => None,
        }
    }
}

const MAX_HOST_LEN: usize = 253;

/// A relay or server address: DNS name, IPv4 or bracketed IPv6, plus a non-zero port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HostPort {
    host: String,
    port: u16,
}

impl HostPort {
    /// The host without IPv6 brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Parse failure for [`HostPort`]. Never includes the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("expected host:port (DNS name, IPv4 or [IPv6], port 1-65535)")]
pub struct HostPortError;

impl FromStr for HostPort {
    type Err = HostPortError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (host, port) = split_host_port(s.trim()).ok_or(HostPortError)?;
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|p| *p != 0)
            .ok_or(HostPortError)?;
        Ok(Self {
            host: host.to_owned(),
            port,
        })
    }
}

/// `[v6]:port` or `name:port`; the host part is checked here.
fn split_host_port(s: &str) -> Option<(&str, &str)> {
    if let Some(rest) = s.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        host.parse::<Ipv6Addr>().ok()?;
        return Some((host, port));
    }
    let (host, port) = s.rsplit_once(':')?;
    is_dns_or_ipv4(host).then_some((host, port))
}

fn is_dns_or_ipv4(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= MAX_HOST_LEN
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

impl fmt::Display for HostPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parses_names_ipv4_and_ipv6() {
        for (input, host, port, shown) in [
            (
                "relay.example.com:21117",
                "relay.example.com",
                21117,
                "relay.example.com:21117",
            ),
            (" 10.0.0.1:1 ", "10.0.0.1", 1, "10.0.0.1:1"),
            ("[::1]:21117", "::1", 21117, "[::1]:21117"),
        ] {
            let hp: HostPort = input.parse().unwrap();
            assert_eq!((hp.host(), hp.port()), (host, port), "{input}");
            assert_eq!(hp.to_string(), shown);
        }
    }

    #[test]
    fn host_port_rejects_malformed() {
        for input in [
            "",
            "host",
            "host:",
            ":80",
            "host:0",
            "host:65536",
            "::1:80",
            "[nothex]:80",
            "a b:80",
            "host:80x",
        ] {
            assert!(input.parse::<HostPort>().is_err(), "{input:?}");
        }
    }

    #[test]
    fn secret_value_json_reveals_only_presence() {
        let v = Value::Secret(Secret::new("hunter2"));
        assert_eq!(v.to_json(), json!({ "set": true }));
        assert_eq!(
            Value::Secret(Secret::default()).to_json(),
            json!({ "set": false })
        );
        assert!(!format!("{v:?}").contains("hunter2"));
    }
}
