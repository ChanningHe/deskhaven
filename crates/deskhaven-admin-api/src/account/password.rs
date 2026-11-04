//! `POST /me/password`: the user's own password change.
//!
//! The current password is verified like a login (same throttle: `pre_check` with the
//! username, a wrong one is a counted failure), but a wrong one is **422**
//! `wrong_password`, never 401 (the console treats 401 as signed out). The new password
//! must pass the policy (422 `validation_failed`, `details.fields.new_password`).
//! Success revokes every session of the user in the change's transaction, records
//! `user.password_change` there, and answers the fresh session's cookies with the user.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{AuthSource, Password};
use garde::Validate;
use serde::Deserialize;
use utoipa::ToSchema;

use super::reauth::{MAX_PASSWORD_INPUT, not_local, user_error, with_current_password};
use crate::audit::{Audit, AuditCtx, Audited};
use crate::auth::{user_agent, with_session};
use crate::me::Me;
use crate::session::{Client, ConsoleUser};
use crate::transport::SecureTransport;
use crate::{AdminApiError, ValidJson};

/// Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PasswordChange {
    #[garde(length(chars, min = 1, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub current_password: String,
    #[garde(length(chars, min = 1, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub new_password: String,
}

/// Changes the signed-in user's password.
#[utoipa::path(
    post,
    path = "/me/password",
    operation_id = "change_password",
    tag = "me",
    security(("console_session" = ["session"])),
    request_body = PasswordChange,
    responses(
        (status = 200, description = "Changed; a fresh session (cookies set)", body = Me),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 409, description = "The account has no local password (`not_local_account`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong current password (`wrong_password`), or the new one breaks the policy (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn change(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    headers: HeaderMap,
    ValidJson(body): ValidJson<PasswordChange>,
) -> Result<Response, AdminApiError> {
    let user_id = principal.user_id;
    let user = state.users.get(user_id).await.map_err(user_error)?;
    if user.auth_source != AuthSource::Local {
        return Err(not_local());
    }
    let audit = ctx.entry(Audit::new("user.password_change").target("user", user_id));
    let current = Password::new(body.current_password.as_str());
    let new = Password::new(body.new_password.as_str());
    let change = state.users.change_password(user_id, &current, &new, &audit);
    let stamp = with_current_password(&state, client, &user.username, change).await?;
    let (session, token) = state
        .sessions
        .create_console(user_id, &stamp, Some(client.addr), user_agent(&headers))
        .await
        .map_err(AdminApiError::internal)?;
    let user = state.users.get(user_id).await.map_err(user_error)?;
    let body = (Audited, Json(Me::of(&state, user)));
    Ok(with_session(&session, &token, body).into_response())
}
