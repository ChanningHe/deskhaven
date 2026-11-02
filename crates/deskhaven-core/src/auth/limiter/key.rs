//! `auth_lockouts` keys of one login attempt.
//!
//! Every attempt touches two rows:
//! - per source: `user:<name>|<ip>`, where `<ip>` is the client's IPv4 address or IPv6 /64
//!   ([`ip_key`]), or `fallback` for a trusted proxy's own address. Exponential backoff.
//! - global: `user:<name>|*`. Higher threshold, fixed duration (`policy.rs`).
//!
//! `<name>` is the `users.username_norm` form of the input ([`username_norm`]), or
//! `~malformed` for input that is not a well-formed username (it cannot name an account).
//! So keys never depend on whether the account exists, attacker-chosen strings neither grow
//! the table nor reach it verbatim (PostgreSQL rejects NUL in text), and every key of one
//! username starts with `user:<name>|`.

use super::ClientAddr;
use super::ip::ip_key;
use crate::users::username_norm;

/// Name part shared by every malformed username (`~` is outside the username charset).
const MALFORMED_NAME: &str = "~malformed";
/// Source part shared by every proxy-fallback address.
const FALLBACK_SOURCE: &str = "fallback";
/// Source part of the global row.
const GLOBAL_SOURCE: &str = "*";
/// `LIKE` escape character for [`username_pattern`].
pub(super) const LIKE_ESCAPE: char = '\\';

/// The username part of a key.
pub(super) fn name_of(username: &str) -> String {
    username_norm(username).unwrap_or_else(|| MALFORMED_NAME.to_owned())
}

/// The source part of a key.
pub(super) fn source_of(client: ClientAddr) -> String {
    if client.is_proxy_fallback {
        FALLBACK_SOURCE.to_owned()
    } else {
        ip_key(client.addr).to_string()
    }
}

/// The global row key of a key name part.
pub(super) fn global_key_of(name: &str) -> String {
    format!("user:{name}|{GLOBAL_SOURCE}")
}

/// `LIKE` pattern (escaped with [`LIKE_ESCAPE`]) matching every key of `name`. Usernames may
/// contain `_`, a `LIKE` wildcard, so it must be escaped.
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

/// The two rows of one attempt.
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

    /// Normalized username (or `~malformed`), for logs.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Client network (or `fallback`), for logs.
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
    use std::net::IpAddr;

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
        assert_eq!(s.source_key(), "user:alice|2001:db8:1:2::");
        assert_eq!(s.global_key(), "user:alice|*");

        let v4 = Subject::new(client("::ffff:192.0.2.7", false), "ALICE");
        assert_eq!(v4.source_key(), "user:alice|192.0.2.7");

        let proxy = Subject::new(client("10.0.0.1", true), "alice");
        assert_eq!(proxy.source_key(), "user:alice|fallback");
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
