//! Insecure-transport detection for the requests that set the console's cookies (login,
//! first-run setup, OIDC start and completion).
//!
//! The session and CSRF cookies are `__Host-` + `Secure` (see `session`), and stay so: a
//! browser on a plain-http page drops them, and the console would loop on 401 right after
//! a "successful" login. Those requests therefore refuse with 403 `insecure_transport`
//! before doing anything, and the console explains the fix: reach it through an https
//! reverse proxy, or through an SSH tunnel to `localhost`. There is no switch to relax
//! the cookies. This is a usability check, not a security control (the cookie attributes
//! are): it only says "secure" on positive evidence.
//!
//! Rule:
//! 1. `Origin` present (browsers send it on every `POST`): the page the browser shows, so
//!    it decides. `https://…` is secure; `http://` to a loopback host (`localhost`,
//!    `*.localhost`, `127.0.0.0/8`, `[::1]`: secure contexts that keep `Secure` cookies)
//!    is secure; anything else is not. This also catches a plain-http address that
//!    bypasses a configured https proxy.
//! 2. No `Origin` (not a browser), in order: the socket peer is a trusted proxy
//!    (`security.trusted_proxies`) that sent `X-Forwarded-Proto`, whose last entry decides
//!    (`https` is secure); else a loopback peer (SSH tunnel, local client) is secure; else
//!    an https `public_url` or `listen.admin_public_url` (TLS ends in front) is secure;
//!    otherwise not.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, header};
use deskhaven_core::state::AppState;
use deskhaven_http::{TrustedProxies, socket_peer};

use crate::origin::TrustedOrigins;
use crate::{AdminApiError, ErrorCode};

/// `X-Forwarded-Proto` (not among the `http` crate's constants).
pub const X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

/// Extractor that refuses (403 `insecure_transport`) a request reaching the console over
/// plain HTTP (see the module docs). Put it before the body in a handler's arguments.
#[derive(Debug, Clone, Copy)]
pub struct SecureTransport;

impl FromRequestParts<Arc<AppState>> for SecureTransport {
    type Rejection = AdminApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let facts = Facts {
            headers: &parts.headers,
            peer: socket_peer(&parts.extensions),
            proxies: parts.extensions.get::<TrustedProxies>(),
            origins: parts.extensions.get::<TrustedOrigins>(),
        };
        if facts.is_secure() {
            Ok(Self)
        } else {
            tracing::debug!(peer = ?facts.peer, "console request over plain HTTP refused");
            Err(insecure())
        }
    }
}

fn insecure() -> AdminApiError {
    AdminApiError::new(
        ErrorCode::InsecureTransport,
        "the console must be reached over https (or through an SSH tunnel to localhost)",
    )
}

/// What the rule looks at.
struct Facts<'a> {
    headers: &'a HeaderMap,
    peer: Option<IpAddr>,
    proxies: Option<&'a TrustedProxies>,
    origins: Option<&'a TrustedOrigins>,
}

impl Facts<'_> {
    fn is_secure(&self) -> bool {
        if let Some(origin) = self.headers.get(header::ORIGIN) {
            return origin.to_str().is_ok_and(origin_is_secure);
        }
        let from_proxy = self
            .peer
            .zip(self.proxies)
            .is_some_and(|(peer, proxies)| proxies.is_trusted(peer));
        if from_proxy && let Some(proto) = forwarded_proto(self.headers) {
            return proto.eq_ignore_ascii_case("https");
        }
        self.peer.is_some_and(|peer| peer.is_loopback())
            || self.origins.is_some_and(TrustedOrigins::any_https)
    }
}

/// The last entry of the last `X-Forwarded-Proto` header (the closest proxy's).
fn forwarded_proto(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get_all(&X_FORWARDED_PROTO).iter().next_back()?;
    let entry = value.to_str().ok()?.rsplit(',').next()?.trim();
    (!entry.is_empty()).then_some(entry)
}

/// An `Origin` value names a page that keeps `Secure` cookies.
fn origin_is_secure(origin: &str) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    if scheme.eq_ignore_ascii_case("https") {
        return true;
    }
    scheme.eq_ignore_ascii_case("http") && is_loopback_host(host_of(authority))
}

/// The host of `host[:port]` / `[v6][:port]`, brackets dropped.
fn host_of(authority: &str) -> &str {
    match authority.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    }
}

fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    fn secure(
        pairs: &[(&str, &str)],
        peer: &str,
        proxies: &TrustedProxies,
        origins: &TrustedOrigins,
    ) -> bool {
        let headers = headers(pairs);
        Facts {
            headers: &headers,
            peer: Some(peer.parse().unwrap()),
            proxies: Some(proxies),
            origins: Some(origins),
        }
        .is_secure()
    }

    const LAN: &str = "192.0.2.10";
    const PROXY: &str = "10.0.0.2";

    fn proxies() -> TrustedProxies {
        TrustedProxies::new(["10.0.0.0/8".parse().unwrap()])
    }

    #[test]
    fn the_browser_origin_decides_when_present() {
        let none = TrustedOrigins::default();
        let https_url = TrustedOrigins::from_urls(["https://rd.example.com"]);
        let p = proxies();
        let secure_origins = [
            "https://rd.example.com",
            "HTTPS://rd.example.com:8443",
            "http://localhost:5173",
            "http://LOCALHOST",
            "http://console.localhost:8080",
            "http://127.0.0.1:21114",
            "http://127.8.9.10",
            "http://[::1]:21114",
        ];
        for origin in secure_origins {
            assert!(secure(&[("origin", origin)], LAN, &p, &none), "{origin}");
        }
        // Plain http elsewhere is insecure even behind a trusted https proxy, from a
        // loopback peer, or with an https public URL: the browser drops the cookie.
        let insecure_origins = [
            "http://192.0.2.10:21114",
            "http://rd.example.com",
            "http://localhost.example.com",
            "http://[::2]",
            "null",
            "ftp://localhost",
        ];
        for origin in insecure_origins {
            let pairs = [("origin", origin), ("x-forwarded-proto", "https")];
            assert!(!secure(&pairs, PROXY, &p, &https_url), "{origin}");
            assert!(!secure(&pairs, "127.0.0.1", &p, &https_url), "{origin}");
        }
    }

    #[test]
    fn without_origin_a_trusted_proxy_header_decides_first() {
        let none = TrustedOrigins::default();
        let https_url = TrustedOrigins::from_urls(["https://rd.example.com"]);
        let p = proxies();

        assert!(secure(&[("x-forwarded-proto", "https")], PROXY, &p, &none));
        assert!(secure(
            &[("x-forwarded-proto", "http, HTTPS")],
            PROXY,
            &p,
            &none
        ));
        assert!(!secure(
            &[("x-forwarded-proto", "http")],
            PROXY,
            &p,
            &https_url
        ));
        // From an untrusted peer the header means nothing.
        assert!(!secure(&[("x-forwarded-proto", "https")], LAN, &p, &none));
    }

    #[test]
    fn without_origin_loopback_peers_and_https_urls_are_secure() {
        let none = TrustedOrigins::default();
        let https_url = TrustedOrigins::from_urls(["", "https://console.example.com/"]);
        let http_url = TrustedOrigins::from_urls(["http://rd.example.com"]);
        let p = proxies();

        assert!(secure(&[], "127.0.0.1", &p, &none));
        assert!(secure(&[], "::1", &p, &none));
        assert!(secure(&[], LAN, &p, &https_url));
        assert!(!secure(&[], LAN, &p, &http_url));
        assert!(!secure(&[], LAN, &p, &none));
        // A trusted proxy without the header: no evidence from it.
        assert!(!secure(&[], PROXY, &p, &none));
    }

    #[test]
    fn no_peer_and_no_configuration_is_insecure() {
        let headers = HeaderMap::new();
        let facts = Facts {
            headers: &headers,
            peer: None,
            proxies: None,
            origins: None,
        };
        assert!(!facts.is_secure());
    }
}
