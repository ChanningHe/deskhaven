//! Real client IP resolution behind trusted reverse proxies (06-security §3.6).
//!
//! # Rule
//!
//! 1. Start with the socket peer, canonicalized (`::ffff:a.b.c.d` -> `a.b.c.d`).
//! 2. If the peer is not inside a trusted CIDR, it is the client; `X-Forwarded-For`
//!    is ignored. With no trusted proxies configured (the default) this always applies.
//! 3. Otherwise walk the `X-Forwarded-For` entries right to left (multiple headers
//!    form one list in arrival order). Each entry must be a bare IPv4/IPv6 address
//!    (surrounding whitespace allowed). The first canonical entry that is not
//!    trusted is the client.
//! 4. If an entry is malformed (empty, has a port, brackets, non-ASCII, ...), stop and
//!    return the last address already validated, i.e. the trusted hop that forwarded
//!    the garbage. If every entry is trusted, the left-most one is the client.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, StatusCode};
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
    pub fn resolve(&self, peer: IpAddr, headers: &HeaderMap) -> IpAddr {
        let mut candidate = peer.to_canonical();
        if !self.is_trusted(candidate) {
            return candidate;
        }
        for value in headers.get_all(X_FORWARDED_FOR).iter().rev() {
            let Ok(list) = value.to_str() else {
                return candidate;
            };
            for entry in list.rsplit(',') {
                let Ok(addr) = entry.trim().parse::<IpAddr>() else {
                    return candidate;
                };
                candidate = addr.to_canonical();
                if !self.is_trusted(candidate) {
                    return candidate;
                }
            }
        }
        candidate
    }
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

/// The resolved client address. Requires `ConnectInfo<SocketAddr>` (serve with
/// `into_make_service_with_connect_info`) and [`TrustedProxies`] in the request
/// extensions (inserted by [`crate::apply_middleware`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

/// Wiring error: a server-side bug, never caused by the client.
#[derive(Debug, thiserror::Error)]
pub enum ClientIpRejection {
    #[error("ConnectInfo<SocketAddr> missing: serve with into_make_service_with_connect_info")]
    MissingConnectInfo,
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

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let ConnectInfo(peer) = ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
            .await
            .map_err(|_| ClientIpRejection::MissingConnectInfo)?;
        let trusted = parts
            .extensions
            .get::<TrustedProxies>()
            .ok_or(ClientIpRejection::MissingTrustedProxies)?;
        Ok(Self(trusted.resolve(peer.ip(), &parts.headers)))
    }
}
