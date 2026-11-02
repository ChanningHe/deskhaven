//! The requesting client's address as the login throttle keys it: the real IP resolved by
//! the HTTP layer (`deskhaven_http::ClientIp`, 06-security §3.6).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_http::ClientIp;

use crate::ClientApiError;

/// Extractor; a missing peer or proxy configuration is a wiring bug (500).
#[derive(Debug, Clone, Copy)]
pub struct Client(pub ClientAddr);

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = ClientApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let ip = ClientIp::from_request_parts(parts, state)
            .await
            .map_err(ClientApiError::internal)?;
        Ok(Self(ClientAddr {
            addr: ip.addr,
            is_proxy_fallback: ip.is_proxy_fallback,
        }))
    }
}
