//! A session is issued only once every factor passed, and every sign-in revokes the session
//! the request presented (no fixation).

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
pub(crate) use oidc::clear_binding_cookie;
pub use oidc::{
    OIDC_BINDING_COOKIE, OidcCompleteRequest, OidcProvider, OidcStart, OidcStartRequest,
};
pub use password::LoginRequest;
pub use totp::TotpLoginRequest;

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
// Not `Debug`: it may carry a challenge secret.
#[derive(Serialize, ToSchema)]
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

pub(crate) fn throttle_error(err: ThrottleError) -> AdminApiError {
    match err {
        ThrottleError::Throttled { retry_after } => AdminApiError::rate_limited(retry_after),
        ThrottleError::Db(db) => AdminApiError::internal(db),
    }
}

pub(crate) fn user_agent(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
}

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

pub(crate) async fn revoke_presented(state: &Arc<AppState>, headers: &HeaderMap) {
    let Some(old) = session::session_token(headers) else {
        return;
    };
    if let Err(err) = state.sessions.revoke_token(old).await {
        tracing::warn!(error = %err, "cannot revoke the session a new sign-in replaces");
    }
}

fn lifetime(session: &Session) -> Duration {
    let ms = session
        .expires_at
        .timestamp_millis()
        .saturating_sub(session.created_at.timestamp_millis());
    Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}
