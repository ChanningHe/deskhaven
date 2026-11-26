//! 1. Start with the socket peer, canonicalized (`::ffff:a.b.c.d` -> `a.b.c.d`).
//! 2. A peer outside every trusted CIDR is the client; `X-Forwarded-For` is ignored.
//! 3. Otherwise walk the `X-Forwarded-For` entries right to left (multiple headers form one
//!    list). Entries are split on `,` at the byte level and judged on their own, so bytes in
//!    entries the walk never reaches cannot affect the result. The first canonical entry
//!    that is not trusted is the client.
//! 4. A malformed reached entry stops the walk at the last validated address: the trusted
//!    hop that forwarded the garbage. If every entry is trusted, the left-most one wins.
//!
//! A trusted result sets [`ClientIp::is_proxy_fallback`]: the address may be shared by many
//! clients, so rate limiting must not lock it out. `::/0` does not match IPv4 or IPv4-mapped
//! peers (they are canonicalized first), which fails safe.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{Extensions, HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use ipnet::{IpNet, Ipv4Net};

pub const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

const MAPPED_V4_PREFIX_LEN: u8 = 96;

/// Empty (the default) = trust none.
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

/// Addresses are canonicalized before matching, so the net must be too.
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

/// Peer of a request dispatched in-process (no socket). Only code inserts it; it never
/// comes from headers. Used as-is: the tunnelled headers are client-controlled, so
/// `X-Forwarded-For` and `x-request-id` are ignored. The dispatcher must target the
/// **client_api router only**, never the admin router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InternalPeer(pub SocketAddr);

pub(crate) fn request_peer(ext: &Extensions) -> Option<(SocketAddr, bool)> {
    if let Some(InternalPeer(addr)) = ext.get() {
        return Some((*addr, true));
    }
    ext.get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr)
        .or_else(|| ext.get::<MockConnectInfo<SocketAddr>>().map(|m| m.0))
        .map(|addr| (addr, false))
}

/// Before any `X-Forwarded-For` resolution; `None` for an in-process request. For decisions
/// about the connection itself (loopback, trusted proxy), not about the client.
pub fn socket_peer(ext: &Extensions) -> Option<IpAddr> {
    match request_peer(ext)? {
        (_, true) => None,
        (addr, false) => Some(addr.ip().to_canonical()),
    }
}

/// Requires a peer and [`TrustedProxies`] in the request extensions (inserted by
/// [`crate::apply_middleware`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp {
    pub addr: IpAddr,
    /// A trusted proxy address: do not lock it out.
    pub is_proxy_fallback: bool,
}

/// A server-side wiring bug, never caused by the client.
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
