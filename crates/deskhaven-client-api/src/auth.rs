//! Any credential problem is one generic 401: the client then resets its login, and the
//! message reveals nothing about why. An uncached token from an address over its bearer
//! budget is a 429 before the database lookup, so random tokens cannot drive one read each;
//! a 429, not a 401, because the token was not checked and a client sharing the address keeps
//! its login.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use deskhaven_core::sessions::{CachedAuth, SessionKind, SessionPrincipal};
use deskhaven_core::state::AppState;

use crate::ClientApiError;
use crate::client::Client;

pub const UNAUTHORIZED_MESSAGE: &str = "Unauthorized";

const SCHEME: &str = "bearer";

/// Storage failures are a 500, never a 401, which would log the client out.
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

pub(crate) fn too_many(retry_after: Duration) -> ClientApiError {
    ClientApiError::TooManyRequests { retry_after }
}

pub(crate) fn unauthorized() -> ClientApiError {
    ClientApiError::Unauthorized(UNAUTHORIZED_MESSAGE.into())
}

/// Several `Authorization` headers are ambiguous and rejected.
pub(crate) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
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
