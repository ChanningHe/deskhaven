//! IPv6 sources are /48, deliberately wider than the IP bucket's /64: a residential customer
//! commonly gets a /56 and a tunnel broker a /48, so per-/64 rows would let one attacker look
//! like hundreds of sources to the global row.
//!
//! Keys never depend on whether the account exists, and attacker-chosen strings never reach
//! the table verbatim: a malformed username maps to `~malformed`.

use std::net::{IpAddr, Ipv6Addr};

use super::ClientAddr;
use crate::users::username_norm;

pub const LOCKOUT_IPV6_PREFIX: u32 = 48;
/// `~` is outside the username charset.
const MALFORMED_NAME: &str = "~malformed";
const FALLBACK_SOURCE: &str = "fallback";
const GLOBAL_SOURCE: &str = "*";
pub(super) const LIKE_ESCAPE: char = '\\';

pub fn source_prefix(addr: IpAddr) -> IpAddr {
    match addr.to_canonical() {
        IpAddr::V6(v6) => {
            let mask = u128::MAX << (128 - LOCKOUT_IPV6_PREFIX);
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
        }
        v4 => v4,
    }
}

pub(super) fn name_of(username: &str) -> String {
    username_norm(username).unwrap_or_else(|| MALFORMED_NAME.to_owned())
}

pub(super) fn source_of(client: ClientAddr) -> String {
    if client.is_proxy_fallback {
        FALLBACK_SOURCE.to_owned()
    } else {
        source_prefix(client.addr).to_string()
    }
}

pub(super) fn global_key_of(name: &str) -> String {
    format!("user:{name}|{GLOBAL_SOURCE}")
}

/// Usernames may contain `_`, a `LIKE` wildcard.
pub(super) fn username_pattern(name: &str) -> String {
    let mut pattern = String::from("user:");
    for c in name.chars() {
        if matches!(c, '%' | '_' | LIKE_ESCAPE) {
            pattern.push(LIKE_ESCAPE);
        }
        pattern.push(c);
    }
    pattern.push_str("|%");
    pattern
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    name: String,
    source: String,
}

impl Subject {
    pub fn new(client: ClientAddr, username: &str) -> Self {
        Self {
            name: name_of(username),
            source: source_of(client),
        }
    }

    /// Stored data can never address the global row or another key shape.
    pub fn from_parts(name: &str, source: &str) -> Option<Self> {
        let name_ok = name == MALFORMED_NAME || username_norm(name).as_deref() == Some(name);
        let source_ok = source == FALLBACK_SOURCE
            || source
                .parse::<IpAddr>()
                .is_ok_and(|ip| source_prefix(ip).to_string() == source);
        (name_ok && source_ok).then(|| Self {
            name: name.to_owned(),
            source: source.to_owned(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn source_key(&self) -> String {
        format!("user:{}|{}", self.name, self.source)
    }

    pub fn global_key(&self) -> String {
        global_key_of(&self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(addr: &str, is_proxy_fallback: bool) -> ClientAddr {
        ClientAddr {
            addr: addr
                .parse()
                .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
            is_proxy_fallback,
        }
    }

    #[test]
    fn keys_combine_normalized_name_and_source() {
        let s = Subject::new(client("2001:db8:1:2:3:4:5:6", false), "Alice");
        assert_eq!(s.source_key(), "user:alice|2001:db8:1::");
        assert_eq!(s.global_key(), "user:alice|*");

        let v4 = Subject::new(client("::ffff:192.0.2.7", false), "ALICE");
        assert_eq!(v4.source_key(), "user:alice|192.0.2.7");

        let proxy = Subject::new(client("10.0.0.1", true), "alice");
        assert_eq!(proxy.source_key(), "user:alice|fallback");
    }

    #[test]
    fn ipv6_sources_are_slash48_and_ipv4_stays_slash32() {
        let ip = |s: &str| {
            s.parse::<IpAddr>()
                .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        };
        assert_eq!(source_prefix(ip("2001:db8:1:ff00::1")), ip("2001:db8:1::"));
        assert_eq!(
            source_prefix(ip("2001:db8:1:ffff:ffff::")),
            ip("2001:db8:1::")
        );
        assert_ne!(
            source_prefix(ip("2001:db8:1::")),
            source_prefix(ip("2001:db8:2::"))
        );
        assert_eq!(source_prefix(ip("192.0.2.7")), ip("192.0.2.7"));
        assert_eq!(source_prefix(ip("::ffff:192.0.2.7")), ip("192.0.2.7"));
    }

    #[test]
    fn malformed_usernames_share_one_name() {
        let long = "a".repeat(65);
        for raw in [
            "",
            "bad name",
            "nul\0byte",
            "ümlaut",
            long.as_str(),
            "~malformed",
        ] {
            assert_eq!(name_of(raw), MALFORMED_NAME, "{raw:?}");
        }
        assert_eq!(name_of(&"a".repeat(64)), "a".repeat(64));
    }

    #[test]
    fn username_pattern_escapes_like_wildcards() {
        assert_eq!(username_pattern("a_b"), "user:a\\_b|%");
        assert_eq!(username_pattern("~malformed"), "user:~malformed|%");
        assert_eq!(username_pattern("x.y@z-w"), "user:x.y@z-w|%");
    }
}
