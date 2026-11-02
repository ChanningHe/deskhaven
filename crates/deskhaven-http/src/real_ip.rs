//! Real client IP resolution behind trusted reverse proxies (06-security §3.6).
//!
//! # Rule
//!
//! 1. Start with the socket peer, canonicalized (`::ffff:a.b.c.d` -> `a.b.c.d`).
//! 2. If the peer is not inside a trusted CIDR, it is the client; `X-Forwarded-For`
//!    is ignored. With no trusted proxies configured (the default) this always applies.
//! 3. Otherwise walk the `X-Forwarded-For` entries right to left. Multiple headers form
//!    one list in arrival order. Entries are split on `,` at the byte level and each is
//!    judged on its own, so bytes in entries the walk never reaches (e.g. obs-text a
//!    client put in front of the proxy-appended address) cannot affect the result.
//!    An entry must be a bare IPv4/IPv6 address, optionally surrounded by spaces/tabs.
//!    The first canonical entry that is not trusted is the client.
//! 4. If a reached entry is malformed (empty, has a port, brackets, non-UTF-8, ...),
//!    stop and return the last address already validated: the trusted hop that
//!    forwarded the garbage. If every entry is trusted, the left-most one is returned.
//!
//! Whenever the result is itself a trusted address (cases 4, a trusted peer without
//! XFF, or an all-trusted chain), [`ClientIp::is_proxy_fallback`] is set: the address
//! may be shared by many clients, so rate limiting must not lock it out.
//!
//! Keep `trusted_proxies` as narrow as possible: every address inside it can choose
//! the client IP. `::/0` does not match IPv4 or IPv4-mapped peers (they are
//! canonicalized to IPv4 first), which fails safe.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{Extensions, HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use ipnet::{IpNet, Ipv4Net};

/// `X-Forwarded-For` (not among the `http` crate's standard header constants).
pub const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

/// IPv4-mapped IPv6 addresses occupy `::ffff:0:0/96`.
const MAPPED_V4_PREFIX_LEN: u8 = 96;

/// CIDRs whose `X-Forwarded-For` is honored. Empty (the default) = trust none.
#[derive(Debug, Clone, Default)]
pub struct TrustedProxies(Arc<[IpNet]>);

impl TrustedProxies {
    pub fn new(nets: impl IntoIterator<Item = IpNet>) -> Self {
        Self(nets.into_iter().map(canonical_net).collect())
    }

    pub fn is_trusted(&self, addr: IpAddr) -> bool {
        let addr = addr.to_canonical();
        self.0.iter().any(|net| net.contains(&addr))
    }

    /// Resolve the client address for a request from `peer` (see module docs).
    pub fn resolve(&self, peer: IpAddr, headers: &HeaderMap) -> ClientIp {
        let mut candidate = peer.to_canonical();
        if !self.is_trusted(candidate) {
            return self.client_ip(candidate);
        }
        let entries = headers
            .get_all(X_FORWARDED_FOR)
            .iter()
            .rev()
            .flat_map(|value| value.as_bytes().rsplit(|b| *b == b','));
        for entry in entries {
            let Some(addr) = parse_entry(entry) else {
                tracing::warn!(%peer, "malformed X-Forwarded-For entry; using the trusted hop");
                break;
            };
            candidate = addr.to_canonical();
            if !self.is_trusted(candidate) {
                break;
            }
        }
        self.client_ip(candidate)
    }

    fn client_ip(&self, addr: IpAddr) -> ClientIp {
        ClientIp {
            addr,
            is_proxy_fallback: self.is_trusted(addr),
        }
    }
}

fn parse_entry(entry: &[u8]) -> Option<IpAddr> {
    let text = std::str::from_utf8(entry).ok()?;
    text.trim_matches([' ', '\t']).parse().ok()
}

/// `::ffff:a.b.c.d/n` (n >= 96) is the IPv4 network `a.b.c.d/(n-96)`; addresses are
/// canonicalized before matching, so the net must be too.
fn canonical_net(net: IpNet) -> IpNet {
    let IpNet::V6(v6) = net else {
        return net;
    };
    match (v6.addr().to_ipv4_mapped(), v6.prefix_len()) {
        (Some(v4), len) if len >= MAPPED_V4_PREFIX_LEN => {
            Ipv4Net::new(v4, len - MAPPED_V4_PREFIX_LEN).map_or(net, IpNet::V4)
        }
        _ => net,
    }
}

/// Peer of a request dispatched in-process (hbbs `HttpProxyRequest`, 03 §1.4),
/// which has no socket. Only code can insert it; it never comes from headers.
///
/// It takes precedence over `ConnectInfo` and is used as the client address
/// as-is: the tunnelled headers are client-controlled, so `X-Forwarded-For` and
/// `x-request-id` are ignored, and it is never a proxy fallback (it is the
/// RustDesk client's own rendezvous connection).
///
/// The in-process dispatcher (M5-07) must target the **client_api router only**
/// (wrapped by [`crate::apply_middleware`]), never the admin router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InternalPeer(pub SocketAddr);

/// The peer a request arrived from, and whether it was in-process.
pub(crate) fn request_peer(ext: &Extensions) -> Option<(SocketAddr, bool)> {
    if let Some(InternalPeer(addr)) = ext.get() {
        return Some((*addr, true));
    }
    ext.get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr)
        .or_else(|| ext.get::<MockConnectInfo<SocketAddr>>().map(|m| m.0))
        .map(|addr| (addr, false))
}

/// The resolved client address. Requires a peer (`ConnectInfo<SocketAddr>` from
/// [`crate::Listeners`], or [`InternalPeer`]) and [`TrustedProxies`] in the
/// request extensions (inserted by [`crate::apply_middleware`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp {
    pub addr: IpAddr,
    /// `addr` is a trusted proxy address (see module docs): do not lock it out.
    pub is_proxy_fallback: bool,
}

/// Wiring error: a server-side bug, never caused by the client.
#[derive(Debug, thiserror::Error)]
pub enum ClientIpRejection {
    #[error("no peer address: serve via deskhaven_http::Listeners or insert InternalPeer")]
    MissingPeer,
    #[error("TrustedProxies extension missing: apply deskhaven_http::apply_middleware")]
    MissingTrustedProxies,
}

impl IntoResponse for ClientIpRejection {
    fn into_response(self) -> Response {
        tracing::error!(error = %self, "cannot resolve client ip");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            crate::INTERNAL_ERROR_BODY,
        )
            .into_response()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = ClientIpRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let (peer, is_internal) =
            request_peer(&parts.extensions).ok_or(ClientIpRejection::MissingPeer)?;
        if is_internal {
            return Ok(Self {
                addr: peer.ip().to_canonical(),
                is_proxy_fallback: false,
            });
        }
        let trusted = parts
            .extensions
            .get::<TrustedProxies>()
            .ok_or(ClientIpRejection::MissingTrustedProxies)?;
        Ok(trusted.resolve(peer.ip(), &parts.headers))
    }
}
