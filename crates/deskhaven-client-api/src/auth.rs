//! `Authorization: Bearer <access_token>` (03-client-api-contract §1.2). Any problem with
//! the credential is a 401 with one generic message: the client then resets its login
//! (§1.3), and the message reveals nothing about why. Tokens are never logged.
//!
//! Token spraying, in order:
//! 1. The session cache answers first (`SessionService::authenticate_cached`): a cached
//!    valid token passes and spends nothing, whatever the address's limit state; a cached
//!    but expired or wrong-kind token (or a malformed one) is a 401 that spends one token.
//! 2. Otherwise the token needs a database lookup. While a recent bearer rejection found the
//!    address over its per-IP budget (`LoginThrottle::check_bearer`; login denials do not
//!    count), that is a 429 **before** the lookup, so random tokens cannot drive one
//!    database read each. A 429 rather than a 401: the token was not checked, and a
//!    legitimate client sharing the address keeps its login (a 401 would reset it).
//! 3. A looked-up token that does not authenticate spends one token of the per-IP budget
//!    logins also draw from (`LoginThrottle::on_bearer_rejected`); none left -> 429.
//!
//! So during an attack from the same address (NAT, or the shared trusted-proxy fallback
//! bucket) users with cached sessions are unaffected; only a cache miss (at most one per
//! session per cache TTL) may get a 429. A missing or malformed header is a plain 401.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use deskhaven_core::sessions::{CachedAuth, SessionKind, SessionPrincipal};
use deskhaven_core::state::AppState;

use crate::ClientApiError;
use crate::client::Client;

/// Body message of every 401.
pub const UNAUTHORIZED_MESSAGE: &str = "Unauthorized";

const SCHEME: &str = "bearer";

/// The client session behind a valid bearer token (the raw token is not kept). As a
/// handler argument it rejects with 401
/// `{"error": "Unauthorized"}` when the header is missing or malformed, or the token is
/// unknown, a console token, revoked, expired or belongs to an inactive user; with 429 +
/// `Retry-After` when an uncached token comes from an address over its per-IP limit (see
/// the module docs); storage failures are a 500 (never a 401, which would log the client
/// out).
#[derive(Clone, Debug)]
pub struct BearerUser(pub SessionPrincipal);

impl FromRequestParts<Arc<AppState>> for BearerUser {
    type Rejection = ClientApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let Client(client) = Client::from_request_parts(parts, state).await?;
        let token = bearer_token(&parts.headers).ok_or_else(unauthorized)?;
        let sessions = &state.sessions;
        let outcome = match sessions
            .authenticate_cached(token, SessionKind::Client)
            .await
        {
            CachedAuth::Hit(outcome) => outcome,
            CachedAuth::Miss => {
                state
                    .login_throttle
                    .check_bearer(client)
                    .map_err(too_many)?;
                sessions.authenticate(token, SessionKind::Client).await
            }
        };
        match outcome {
            Ok(principal) => Ok(Self(principal)),
            Err(err) if err.is_unauthenticated() => {
                tracing::debug!(reason = %err, "bearer token rejected");
                state
                    .login_throttle
                    .on_bearer_rejected(client)
                    .map_err(too_many)?;
                Err(unauthorized())
            }
            Err(err) => Err(ClientApiError::internal(err)),
        }
    }
}

fn too_many(retry_after: Duration) -> ClientApiError {
    ClientApiError::TooManyRequests { retry_after }
}

pub(crate) fn unauthorized() -> ClientApiError {
    ClientApiError::Unauthorized(UNAUTHORIZED_MESSAGE.into())
}

/// The token of the one `Authorization` header: scheme `Bearer` (case-insensitive), one or
/// more spaces (RFC 7235 `1*SP`), then exactly one non-empty token without whitespace.
/// Several `Authorization` headers are ambiguous and rejected.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, rest) = value.to_str().ok()?.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return None;
    }
    let token = rest.trim_start_matches(' ');
    let is_single = !token.is_empty() && !token.contains(|c: char| c.is_ascii_whitespace());
    is_single.then_some(token)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(values: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for v in values {
            map.append(header::AUTHORIZATION, HeaderValue::from_str(v).unwrap());
        }
        map
    }

    #[test]
    fn accepts_the_bearer_scheme_in_any_case_with_one_token() {
        for (value, token) in [
            ("Bearer abc", "abc"),
            ("bearer abc", "abc"),
            ("BEARER abc", "abc"),
            ("Bearer   abc", "abc"),
        ] {
            assert_eq!(bearer_token(&headers(&[value])), Some(token), "{value:?}");
        }
    }

    #[test]
    fn rejects_missing_malformed_and_ambiguous_headers() {
        assert_eq!(bearer_token(&HeaderMap::new()), None);
        for value in [
            "",
            "Bearer",
            "Bearer ",
            "Bearer    ",
            "Basic abc",
            "Bearerabc",
            "Token abc",
            "Bearer abc def",
            "Bearer abc ",
            "Bearer\tabc",
            "Bearer abc\tdef",
        ] {
            assert_eq!(bearer_token(&headers(&[value])), None, "{value:?}");
        }
        assert_eq!(bearer_token(&headers(&["Bearer a", "Bearer b"])), None);
    }

    #[test]
    fn rejects_non_ascii_header_values() {
        let mut map = HeaderMap::new();
        let value = HeaderValue::from_bytes(b"Bearer \xffabc").unwrap();
        map.insert(header::AUTHORIZATION, value);

        assert_eq!(bearer_token(&map), None);
    }
}
