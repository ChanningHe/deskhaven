//! Console sign-in and sign-out (06 §3, M3-02).
//!
//! | method | path | permission | what |
//! |---|---|---|---|
//! | `POST` | `/auth/login` | public | password; signed in, or a TOTP challenge |
//! | `POST` | `/auth/login/totp` | public | the challenge with a TOTP or recovery code |
//! | `POST` | `/auth/logout` | public | revokes the presented session, clears the cookies |
//! | `GET` | `/auth/oidc/providers` | public | the OIDC login buttons |
//! | `POST` | `/auth/oidc/start` | public | a console OIDC flow; the IdP URL and a binding cookie |
//! | `POST` | `/auth/oidc/complete` | public | the IdP's answer, from the browser that started |
//!
//! Every sign-in requires a secure transport (`transport`), mints a fresh console session
//! (06 §3.2; a session the request presented is revoked: no fixation) and answers
//! [`LoginResponse::SignedIn`] with the session and CSRF cookies. A session is only ever
//! issued once every factor passed: with TOTP enabled the password step returns a
//! challenge and no cookie. A session whose account setup is unfinished is still issued,
//! but restricted at the gate (`account_setup`); `user.setup_required` tells the console
//! where to go.
//!
//! Login steps follow the login throttle's protocol (`deskhaven_core::auth::limiter`) and
//! write `login_events` with `client = console` (see `event`).

mod event;
mod failure;
mod logout;
mod oidc;
mod password;
mod totp;

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use deskhaven_core::sessions::{Session, SessionToken};
use deskhaven_core::state::AppState;
use deskhaven_core::users::User;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::routes;

use crate::AdminApiError;
use crate::me::Me;
use crate::routes::{AdminRoutes, RouteError};
use crate::session::{self, session_cookies};
use deskhaven_core::auth::limiter::ThrottleError;

pub(crate) use failure::BUSY_RETRY_AFTER;
pub use oidc::{
    OIDC_BINDING_COOKIE, OidcCompleteRequest, OidcProvider, OidcStart, OidcStartRequest,
};
pub use password::LoginRequest;
pub use totp::TotpLoginRequest;

/// The console's sign-in routes.
pub(crate) fn routes() -> Result<AdminRoutes, RouteError> {
    AdminRoutes::new()
        .routes(routes!(password::login))?
        .routes(routes!(totp::login))?
        .routes(routes!(logout::logout))?
        .routes(routes!(oidc::providers))?
        .routes(routes!(oidc::start))?
        .routes(routes!(oidc::complete))
}

/// The answer of a sign-in step.
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LoginResponse {
    /// Signed in: the session and CSRF cookies are set. When `user.setup_required` is set,
    /// the session reaches only `/me`, the password change, the TOTP enrolment and logout
    /// until that step is done.
    SignedIn { user: Me },
    /// The password was right and the account has TOTP: send a TOTP or recovery code with
    /// this challenge to `POST /auth/login/totp` (valid 5 minutes, 5 codes). No cookie.
    TotpRequired { challenge: String },
}

/// A refusal of the login throttle as an API error.
pub(crate) fn throttle_error(err: ThrottleError) -> AdminApiError {
    match err {
        ThrottleError::Throttled { retry_after } => AdminApiError::rate_limited(retry_after),
        ThrottleError::Db(db) => AdminApiError::internal(db),
    }
}

/// The `User-Agent` of a request, stored with its session.
pub(crate) fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

/// `body` with the cookies of the new console session.
pub(crate) fn with_session(
    session: &Session,
    token: &SessionToken,
    body: impl IntoResponse,
) -> Response {
    let cookies = session_cookies(token, lifetime(session));
    (
        AppendHeaders(cookies.map(|value| (header::SET_COOKIE, value))),
        [(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))],
        body,
    )
        .into_response()
}

/// The signed-in answer for `user` with its new session's cookies.
pub(crate) fn signed_in(
    state: &AppState,
    user: User,
    session: &Session,
    token: &SessionToken,
) -> Response {
    let body = Json(LoginResponse::SignedIn {
        user: Me::of(state, user),
    });
    with_session(session, token, body)
}

/// Revokes the session a sign-in request presented (it is replaced). Best effort.
pub(crate) async fn revoke_presented(state: &Arc<AppState>, headers: &HeaderMap) {
    let Some(old) = session::session_token(headers) else {
        return;
    };
    if let Err(err) = state.sessions.revoke_token(old).await {
        tracing::warn!(error = %err, "cannot revoke the session a new sign-in replaces");
    }
}

/// The absolute lifetime of a session, for the cookies' `Max-Age`.
fn lifetime(session: &Session) -> Duration {
    let ms = session
        .expires_at
        .timestamp_millis()
        .saturating_sub(session.created_at.timestamp_millis());
    Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}
