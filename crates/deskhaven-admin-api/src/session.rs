//! `__Host-` forbids a `Domain` attribute, so a sibling subdomain cannot plant the cookies.
//! CSRF is a double submit checked against the session cookie, never the CSRF cookie: a
//! planted CSRF cookie cannot match a session it was not derived from. Over plain HTTP the
//! requests that set cookies refuse with `insecure_transport` instead of relaxing them.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, header};
use base64_022::Engine as _;
use base64_022::engine::general_purpose::URL_SAFE_NO_PAD;
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::sessions::{CachedAuth, SessionKind, SessionPrincipal, SessionToken};
use deskhaven_core::state::AppState;
use deskhaven_http::ClientIp;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::AdminApiError;
use crate::account_setup::{pending_step, setup_required};

pub const SESSION_COOKIE: &str = "__Host-deskhaven_session";
pub const CSRF_COOKIE: &str = "__Host-deskhaven_csrf";
pub const CSRF_HEADER: HeaderName = HeaderName::from_static("x-deskhaven-csrf");

const CSRF_DOMAIN: &[u8] = b"deskhaven/csrf/v1\0";
const COOKIE_ATTRIBUTES: &str = "Path=/; Secure; SameSite=Lax";

/// On a `Public` route it authenticates the session cookie itself (the gate has checked
/// origin and CSRF).
#[derive(Clone, Debug)]
pub struct ConsoleUser(pub SessionPrincipal);

impl FromRequestParts<Arc<AppState>> for ConsoleUser {
    type Rejection = AdminApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        if let Some(user) = parts.extensions.get::<Self>() {
            return Ok(user.clone());
        }
        let client = client_addr(parts, state).await?;
        let token = session_token(&parts.headers).ok_or_else(AdminApiError::unauthorized)?;
        let principal = authenticate(state, client, token).await?;
        // A `Public` route: the gate did not check the account setup, so do it here (a
        // restricted session reaches only `Session` routes through this extractor).
        if let Some(step) = pending_step(&state.tfa, principal.role, principal.account) {
            return Err(setup_required(step));
        }
        if let Some(SessionSlot(slot)) = parts.extensions.get::<SessionSlot>() {
            // Only the first extraction fills it; later ones see the same principal.
            let _ = slot.set(principal.user_id);
        }
        parts.extensions.insert(Self(principal.clone()));
        Ok(Self(principal))
    }
}

/// Lets the gate of a `Public` route attribute its audit row to the user a handler
/// authenticated.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionSlot(pub Arc<OnceLock<i64>>);

/// A missing peer is a wiring bug (500).
pub(crate) async fn client_addr<S: Send + Sync>(
    parts: &mut Parts,
    state: &S,
) -> Result<ClientAddr, AdminApiError> {
    let ip = ClientIp::from_request_parts(parts, state)
        .await
        .map_err(AdminApiError::internal)?;
    Ok(ClientAddr {
        addr: ip.addr,
        is_proxy_fallback: ip.is_proxy_fallback,
    })
}

pub(crate) async fn authenticate(
    state: &AppState,
    client: ClientAddr,
    token: &str,
) -> Result<SessionPrincipal, AdminApiError> {
    let sessions = &state.sessions;
    let throttle = &state.login_throttle;
    let outcome = match sessions
        .authenticate_cached(token, SessionKind::Console)
        .await
    {
        CachedAuth::Hit(outcome) => outcome,
        CachedAuth::Miss => {
            throttle
                .check_bearer(client)
                .map_err(AdminApiError::rate_limited)?;
            sessions.authenticate(token, SessionKind::Console).await
        }
    };
    match outcome {
        Ok(principal) => Ok(principal),
        Err(err) if err.is_unauthenticated() => {
            tracing::debug!(reason = %err, "console session rejected");
            throttle
                .on_bearer_rejected(client)
                .map_err(AdminApiError::rate_limited)?;
            Err(AdminApiError::unauthorized())
        }
        Err(err) => Err(AdminApiError::internal(err)),
    }
}

/// `None` when absent, empty or present more than once (ambiguous).
pub(crate) fn session_token(headers: &HeaderMap) -> Option<&str> {
    cookie(headers, SESSION_COOKIE)
}

/// `None` when absent, empty or present more than once (ambiguous).
pub(crate) fn cookie<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    let mut found = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .filter(|(found, _)| *found == name)
        .map(|(_, value)| value);
    let value = found.next()?;
    (found.next().is_none() && !value.is_empty()).then_some(value)
}

#[derive(Debug, Clone, Copy)]
pub struct Client(pub ClientAddr);

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = AdminApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        client_addr(parts, state).await.map(Self)
    }
}

pub fn csrf_token(session_token: &str) -> String {
    let digest = Sha256::new()
        .chain_update(CSRF_DOMAIN)
        .chain_update(session_token.as_bytes())
        .finalize();
    URL_SAFE_NO_PAD.encode(digest)
}

pub(crate) fn is_state_changing(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

pub(crate) fn check_csrf(
    headers: &HeaderMap,
    session_token: Option<&str>,
) -> Result<(), AdminApiError> {
    let mut values = headers.get_all(&CSRF_HEADER).iter();
    let (Some(presented), None) = (values.next(), values.next()) else {
        return Err(AdminApiError::csrf_failed());
    };
    let matches = match session_token {
        Some(token) => bool::from(presented.as_bytes().ct_eq(csrf_token(token).as_bytes())),
        None => !presented.is_empty(),
    };
    if matches {
        Ok(())
    } else {
        Err(AdminApiError::csrf_failed())
    }
}

/// Both expire after `max_age`: use the session's absolute lifetime.
pub fn session_cookies(token: &SessionToken, max_age: Duration) -> [HeaderValue; 2] {
    let secs = max_age.as_secs();
    let session = format!(
        "{SESSION_COOKIE}={}; {COOKIE_ATTRIBUTES}; HttpOnly; Max-Age={secs}",
        token.expose()
    );
    let csrf = format!(
        "{CSRF_COOKIE}={}; {COOKIE_ATTRIBUTES}; Max-Age={secs}",
        csrf_token(token.expose())
    );
    [session, csrf].map(cookie_header)
}

pub(crate) fn http_only_cookie(name: &str, value: &str, max_age: Duration) -> HeaderValue {
    let secs = max_age.as_secs();
    cookie_header(format!(
        "{name}={value}; {COOKIE_ATTRIBUTES}; HttpOnly; Max-Age={secs}"
    ))
}

pub fn clear_session_cookies() -> [HeaderValue; 2] {
    [
        format!("{SESSION_COOKIE}=; {COOKIE_ATTRIBUTES}; HttpOnly; Max-Age=0"),
        format!("{CSRF_COOKIE}=; {COOKIE_ATTRIBUTES}; Max-Age=0"),
    ]
    .map(cookie_header)
}

/// Tokens and CSRF values are base64url and the attributes are constant, so this never
/// falls back; the fallback would only drop the cookie, never emit a malformed one.
fn cookie_header(value: String) -> HeaderValue {
    HeaderValue::from_str(&value).unwrap_or_else(|_| HeaderValue::from_static(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookies(lines: &[&str]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for line in lines {
            map.append(header::COOKIE, HeaderValue::from_str(line).unwrap());
        }
        map
    }

    #[test]
    fn finds_the_session_cookie_among_others_and_across_headers() {
        let one = cookies(&["a=1; __Host-deskhaven_session=tok; b=2"]);
        let split = cookies(&["a=1", "__Host-deskhaven_session=tok"]);

        assert_eq!(session_token(&one), Some("tok"));
        assert_eq!(session_token(&split), Some("tok"));
    }

    #[test]
    fn missing_empty_or_duplicated_session_cookies_are_none() {
        for lines in [
            &[][..],
            &["a=1"],
            &["__Host-deskhaven_session="],
            &["deskhaven_session=tok"],
            &["__Host-deskhaven_session=a; __Host-deskhaven_session=b"],
            &["__Host-deskhaven_session=a", "__Host-deskhaven_session=b"],
        ] {
            assert_eq!(session_token(&cookies(lines)), None, "{lines:?}");
        }
    }

    #[test]
    fn csrf_token_is_derived_deterministically_and_differs_per_session() {
        let a = csrf_token("session-a");

        assert_eq!(a, csrf_token("session-a"));
        assert_ne!(a, csrf_token("session-b"));
        assert_eq!(a.len(), 43);
        assert!(!a.contains("session-a"));
    }

    #[test]
    fn csrf_check_needs_exactly_one_matching_header() {
        let token = "session";
        let good = HeaderValue::from_str(&csrf_token(token)).unwrap();
        let mut ok = HeaderMap::new();
        ok.insert(CSRF_HEADER, good.clone());
        let mut twice = ok.clone();
        twice.append(CSRF_HEADER, good);
        let mut wrong = HeaderMap::new();
        wrong.insert(CSRF_HEADER, HeaderValue::from_static("nope"));

        assert!(check_csrf(&ok, Some(token)).is_ok());
        for headers in [HeaderMap::new(), twice.clone(), wrong.clone()] {
            let err = check_csrf(&headers, Some(token)).unwrap_err();
            assert_eq!(err.code(), crate::ErrorCode::CsrfFailed);
        }
        // Without a session any single non-empty value passes.
        let mut empty = HeaderMap::new();
        empty.insert(CSRF_HEADER, HeaderValue::from_static(""));
        assert!(check_csrf(&wrong, None).is_ok());
        for headers in [HeaderMap::new(), twice, empty] {
            assert!(check_csrf(&headers, None).is_err());
        }
    }

    #[test]
    fn only_safe_methods_skip_csrf() {
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(!is_state_changing(&method), "{method}");
        }
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(is_state_changing(&method), "{method}");
        }
    }

    #[test]
    fn clearing_cookies_keeps_the_attributes() {
        let [session, csrf] = clear_session_cookies();

        assert_eq!(
            session,
            "__Host-deskhaven_session=; Path=/; Secure; SameSite=Lax; HttpOnly; Max-Age=0"
        );
        assert_eq!(
            csrf,
            "__Host-deskhaven_csrf=; Path=/; Secure; SameSite=Lax; Max-Age=0"
        );
    }
}
