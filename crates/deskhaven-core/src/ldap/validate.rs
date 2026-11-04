//! Field rules of a directory. TLS by default (06 §3.5): `ldaps://`, or `ldap://` with
//! StartTLS; plain `ldap://` only to a loopback host (a local test directory).

use std::net::IpAddr;

use url::{Host, Url};

use super::error::DirectoryError;
use super::filter;
use super::model::{DEFAULT_USER_FILTER, DEFAULT_USERNAME_ATTR};
use crate::text;

const MAX_NAME_LEN: usize = 64;
const MAX_URL_LEN: usize = 2048;
const MAX_DN_LEN: usize = 1024;
const MAX_FILTER_LEN: usize = 1024;
const MAX_ATTR_LEN: usize = 64;
const MAX_BIND_PASSWORD_LEN: usize = 1024;

pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> DirectoryError {
    DirectoryError::Invalid {
        field,
        reason: reason.into(),
    }
}

fn plain(field: &'static str, value: &str, max: usize) -> Result<String, DirectoryError> {
    match text::problem(value, max, false) {
        Some(reason) => Err(invalid(field, reason)),
        None => Ok(value.to_owned()),
    }
}

pub(crate) fn name(raw: &str) -> Result<String, DirectoryError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid("name", "must not be empty"));
    }
    plain("name", trimmed, MAX_NAME_LEN)
}

pub(crate) fn url(raw: &str, start_tls: bool) -> Result<(), DirectoryError> {
    let bad = |reason: &str| Err(invalid("url", reason));
    if raw.len() > MAX_URL_LEN {
        return bad("is too long");
    }
    let Ok(parsed) = Url::parse(raw) else {
        return bad("is not a URL");
    };
    if !matches!(parsed.scheme(), "ldap" | "ldaps") {
        return bad("must be ldap:// or ldaps://");
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return bad("must name a host");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return bad("must not carry credentials");
    }
    let bare = matches!(parsed.path(), "" | "/");
    if !bare || parsed.query().is_some() || parsed.fragment().is_some() {
        return bad("must be scheme, host and port only");
    }
    match (parsed.scheme(), start_tls) {
        ("ldaps", true) => Err(invalid(
            "start_tls",
            "is for ldap:// URLs (ldaps:// is TLS already)",
        )),
        ("ldap", false) if !is_loopback(&parsed) => bad(
            "plain ldap:// needs StartTLS (or use ldaps://); only a loopback host may go without",
        ),
        _ => Ok(()),
    }
}

/// `ldap:` is not a special scheme, so `url` leaves IPv4 literals as domain strings.
fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(name)) => {
            name.eq_ignore_ascii_case("localhost")
                || name.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
        }
        Some(Host::Ipv4(addr)) => IpAddr::V4(addr).is_loopback(),
        Some(Host::Ipv6(addr)) => IpAddr::V6(addr).is_loopback(),
        None => false,
    }
}

pub(crate) fn dn(field: &'static str, raw: &str) -> Result<String, DirectoryError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid(field, "must not be empty"));
    }
    plain(field, trimmed, MAX_DN_LEN)
}

/// Blank is none.
pub(crate) fn optional_dn(
    field: &'static str,
    raw: Option<&str>,
) -> Result<Option<String>, DirectoryError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(value) => dn(field, value).map(Some),
    }
}

fn checked_filter(field: &'static str, value: &str) -> Result<String, DirectoryError> {
    let value = plain(field, value, MAX_FILTER_LEN)?;
    // ldap3 also parses a bare `attr=value`; RFC 4515 wants the parentheses (and the
    // filter is spliced into `(&...)`).
    let parenthesized = value.starts_with('(') && value.ends_with(')');
    if !parenthesized || !filter::is_valid_filter(&value) {
        return Err(invalid(
            field,
            "is not a valid LDAP filter, e.g. (objectClass=person)",
        ));
    }
    Ok(value)
}

/// Blank: [`DEFAULT_USER_FILTER`].
pub(crate) fn user_filter(raw: &str) -> Result<String, DirectoryError> {
    match raw.trim() {
        "" => Ok(DEFAULT_USER_FILTER.to_owned()),
        value => checked_filter("user_filter", value),
    }
}

/// Blank is none. Checked with sample values in its placeholders.
pub(crate) fn group_filter(raw: Option<&str>) -> Result<Option<String>, DirectoryError> {
    let Some(value) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let sample = filter::group_search(value, "cn=sample,dc=example", "sample");
    checked_filter("group_filter", &sample)?;
    plain("group_filter", value, MAX_FILTER_LEN).map(Some)
}

/// An attribute description as RFC 4512 names it: a name (`[A-Za-z][A-Za-z0-9-]*`) or a
/// numeric OID.
fn is_attr(value: &str) -> bool {
    let name = value.starts_with(|c: char| c.is_ascii_alphabetic())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let oid = value.split('.').all(|n| {
        !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && (n == "0" || !n.starts_with('0'))
    });
    value.len() <= MAX_ATTR_LEN && (name || oid)
}

fn attr(field: &'static str, value: &str) -> Result<String, DirectoryError> {
    if is_attr(value) {
        Ok(value.to_owned())
    } else {
        Err(invalid(
            field,
            "must be an attribute name such as uid, or a numeric OID",
        ))
    }
}

/// Blank: [`DEFAULT_USERNAME_ATTR`].
pub(crate) fn username_attr(raw: &str) -> Result<String, DirectoryError> {
    match raw.trim() {
        "" => Ok(DEFAULT_USERNAME_ATTR.to_owned()),
        value => attr("username_attr", value),
    }
}

/// Blank is none.
pub(crate) fn optional_attr(
    field: &'static str,
    raw: Option<&str>,
) -> Result<Option<String>, DirectoryError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(value) => attr(field, value).map(Some),
    }
}

/// Blank is none. Not trimmed: passwords are opaque.
pub(crate) fn bind_password(raw: Option<&str>) -> Result<Option<String>, DirectoryError> {
    match raw.filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(secret)
            if secret.len() > MAX_BIND_PASSWORD_LEN || secret.chars().any(char::is_control) =>
        {
            Err(invalid(
                "bind_password",
                format!("must be at most {MAX_BIND_PASSWORD_LEN} bytes without control characters"),
            ))
        }
        Some(secret) => Ok(Some(secret.to_owned())),
    }
}

/// A service account needs a password, and a password needs an account.
pub(crate) fn bind_pair(bind_dn: bool, has_password: bool) -> Result<(), DirectoryError> {
    match (bind_dn, has_password) {
        (true, false) => Err(invalid("bind_password", "is required with a bind DN")),
        (false, true) => Err(invalid("bind_dn", "is required with a bind password")),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::filter::DEFAULT_GROUP_FILTER;
    use super::*;

    #[test]
    fn the_default_group_filter_is_accepted() {
        assert!(group_filter(Some(DEFAULT_GROUP_FILTER)).unwrap().is_some());
    }

    #[test]
    fn attribute_names_follow_rfc_4512() {
        for ok in [
            "uid",
            "sAMAccountName",
            "x-custom-1",
            "2.5.4.3",
            "0.9.2342.19200300.100.1.1",
        ] {
            assert!(is_attr(ok), "{ok}");
        }
        for bad in [
            "",
            "1uid",
            "u id",
            "uid;binary",
            "01.2",
            "2..5",
            "uid*",
            &"a".repeat(65),
        ] {
            assert!(!is_attr(bad), "{bad}");
        }
    }

    #[test]
    fn urls_take_scheme_host_and_port_only() {
        assert!(url("ldaps://ldap.example.org:636", false).is_ok());
        assert!(url("ldaps://ldap.example.org/", false).is_ok());
        assert!(url("ldap://[::1]:389", false).is_ok());
        assert!(url("ldap://localhost", false).is_ok());
        assert!(url("ldap://127.0.0.1:3389", false).is_ok());
        assert!(url("ldap://10.0.0.1", false).is_err());
        assert!(url("ldap://localhost.example.org", false).is_err());
    }
}
