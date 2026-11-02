//! `POST /auth/logout`: revokes the presented console session (06 §3.2: immediately) and
//! clears its cookies (session, CSRF and a pending OIDC binding). Public, so it also clears
//! the cookies of a session that is already gone (the gate still requires the CSRF header and a same-origin request). Audited as
//! `session.logout` by the owner of a session it revoked.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Response};
use deskhaven_core::console_audit::Actor;
use deskhaven_core::state::AppState;

use super::oidc::clear_binding_cookie;
use crate::audit::Audit;
use crate::session::{self, Client, clear_session_cookies};
use crate::transport::SecureTransport;
use crate::{AdminApiError, ErrorCode};

/// Signs out.
#[utoipa::path(
    post,
    path = "/auth/logout",
    operation_id = "logout",
    tag = "auth",
    security(()),
    responses(
        (status = 204, description = "Signed out; the cookies are cleared"),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many rejected sessions from this address (`rate_limited`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn logout(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    Client(client): Client,
    headers: HeaderMap,
) -> Result<Response, AdminApiError> {
    let audit = match session::session_token(&headers) {
        None => None,
        Some(token) => match session::authenticate(&state, client, token).await {
            Ok(principal) => {
                state
                    .sessions
                    .revoke(principal.session_id)
                    .await
                    .map_err(AdminApiError::internal)?;
                let audit = Audit::new("session.logout")
                    .actor(Actor::User(principal.user_id))
                    .target("session", principal.session_id);
                Some(audit)
            }
            Err(err) if err.code() == ErrorCode::Unauthorized => None,
            Err(err) => return Err(err),
        },
    };
    let [session, csrf] = clear_session_cookies();
    let cookies = [session, csrf, clear_binding_cookie()].map(|value| (header::SET_COOKIE, value));
    let mut response = (StatusCode::NO_CONTENT, AppendHeaders(cookies)).into_response();
    if let Some(audit) = audit {
        response.extensions_mut().insert(audit);
    }
    Ok(response)
}
