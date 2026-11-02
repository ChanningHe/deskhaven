//! `Authorization: Bearer <access_token>` (03-client-api-contract §1.2). Any problem with
//! the credential is a 401 with one generic message: the client then resets its login
//! (§1.3), and the message reveals nothing about why. Tokens are never logged.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use deskhaven_core::sessions::SessionPrincipal;
use deskhaven_core::state::AppState;

use crate::ClientApiError;

/// Body message of every 401.
pub const UNAUTHORIZED_MESSAGE: &str = "Unauthorized";

const SCHEME: &str = "bearer";

/// The session behind a valid bearer token. As a handler argument it rejects with
/// 401 `{"error": "Unauthorized"}` when the header is missing or malformed, or the token
/// is unknown, revoked, expired or belongs to an inactive user; storage failures are a 500
/// (never a 401, which would log the client out).
#[derive(Clone, Debug)]
pub struct BearerUser(pub SessionPrincipal);

impl FromRequestParts<Arc<AppState>> for BearerUser {
    type Rejection = ClientApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers).ok_or_else(unauthorized)?;
        match state.sessions.authenticate(token).await {
            Ok(principal) => Ok(Self(principal)),
            Err(err) if err.is_unauthenticated() => {
                tracing::debug!(reason = %err, "bearer token rejected");
                Err(unauthorized())
            }
            Err(err) => Err(ClientApiError::internal(err)),
        }
    }
}

fn unauthorized() -> ClientApiError {
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
