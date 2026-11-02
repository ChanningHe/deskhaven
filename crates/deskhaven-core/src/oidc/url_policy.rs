//! Which URLs deskhaven talks to, or sends a browser to, for a provider.
//!
//! Provider URLs are admin-configured, but the endpoints come from the IdP's discovery
//! document, so they are checked as untrusted input:
//!
//! - https, always;
//! - plain http only when the issuer itself is plain http on a loopback host (a local test
//!   or development IdP), and then only to loopback hosts. An https issuer can never steer
//!   requests to plain http (no downgrade, no reaching local plaintext services);
//! - never credentials in the URL; issuers additionally take no query or fragment.

use std::net::IpAddr;

use url::{Host, Url};

use super::resolver::is_forbidden;

/// Longest accepted issuer URL.
pub const MAX_ISSUER_LEN: usize = 2048;

/// The rule for one provider, derived from its issuer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EndpointPolicy {
    allow_http_loopback: bool,
}

impl EndpointPolicy {
    /// For an issuer accepted by [`check_issuer`].
    pub(crate) fn for_issuer(issuer: &Url) -> Self {
        Self {
            allow_http_loopback: issuer.scheme() == "http",
        }
    }

    pub(crate) fn check(self, url: &Url) -> Result<(), &'static str> {
        if !url.username().is_empty() || url.password().is_some() {
            return Err("credentials in the URL are not allowed");
        }
        // Names are filtered by the resolver; IP literals never reach it.
        let literal = match url.host() {
            Some(Host::Ipv4(addr)) => Some(IpAddr::V4(addr)),
            Some(Host::Ipv6(addr)) => Some(IpAddr::V6(addr)),
            _ => None,
        };
        if literal.is_some_and(is_forbidden) {
            return Err("link-local and cloud metadata addresses are not allowed");
        }
        match url.scheme() {
            "https" if url.host().is_some() => Ok(()),
            "http" if self.allow_http_loopback && is_loopback(url) => Ok(()),
            "http" => Err("plain http is only allowed to loopback hosts of a loopback issuer"),
            _ => Err("the URL must be https with a host"),
        }
    }
}

/// An issuer the provider service accepts: https with a host, or http on a loopback host;
/// no credentials, query or fragment.
pub(crate) fn check_issuer(raw: &str) -> Result<Url, &'static str> {
    if raw.len() > MAX_ISSUER_LEN {
        return Err("must be at most 2048 characters");
    }
    let url = Url::parse(raw).map_err(|_| "must be an absolute URL")?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err("must not have a query or fragment");
    }
    let loopback_http = EndpointPolicy {
        allow_http_loopback: true,
    };
    loopback_http.check(&url)?;
    Ok(url)
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(addr)) => IpAddr::V4(addr).is_loopback(),
        Some(Host::Ipv6(addr)) => IpAddr::V6(addr).is_loopback(),
        Some(Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn issuers_are_https_or_loopback_http_without_extras() {
        for ok in [
            "https://idp.example.com",
            "https://idp.example.com/realms/main",
            "http://127.0.0.1:8080",
            "http://[::1]:9000/issuer",
            "http://localhost:8080",
        ] {
            assert!(check_issuer(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://idp.example.com",
            "http://10.0.0.1",
            "ftp://idp.example.com",
            "https://user:pw@idp.example.com",
            "https://idp.example.com/?a=1",
            "https://idp.example.com/#x",
            "idp.example.com",
            "javascript:alert(1)",
        ] {
            assert!(check_issuer(bad).is_err(), "{bad}");
        }
        assert!(
            check_issuer(&format!("https://a.example/{}", "x".repeat(MAX_ISSUER_LEN))).is_err()
        );
    }

    #[test]
    fn an_https_issuer_never_allows_plain_http_endpoints() {
        let policy = EndpointPolicy::for_issuer(&url("https://idp.example.com"));

        assert!(policy.check(&url("https://keys.example.net/jwks")).is_ok());
        assert!(policy.check(&url("http://127.0.0.1:6379/")).is_err());
        assert!(policy.check(&url("http://idp.example.com/token")).is_err());
        assert!(policy.check(&url("file:///etc/passwd")).is_err());
        assert!(
            policy
                .check(&url("https://169.254.169.254/latest"))
                .is_err()
        );
        assert!(policy.check(&url("https://[fd00:ec2::254]/")).is_err());
        assert!(policy.check(&url("https://100.100.100.200/")).is_err());
        assert!(check_issuer("https://169.254.169.254").is_err());
        assert!(
            policy
                .check(&url("https://u:p@idp.example.com/token"))
                .is_err()
        );
    }

    #[test]
    fn a_loopback_issuer_allows_loopback_http_only() {
        let policy = EndpointPolicy::for_issuer(&url("http://127.0.0.1:8080"));

        assert!(policy.check(&url("http://127.0.0.1:8080/token")).is_ok());
        assert!(policy.check(&url("http://localhost:1/jwks")).is_ok());
        assert!(policy.check(&url("http://192.168.1.1/token")).is_err());
        assert!(policy.check(&url("https://idp.example.com/token")).is_ok());
    }
}
