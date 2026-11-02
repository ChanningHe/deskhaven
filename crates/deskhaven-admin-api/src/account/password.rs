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
use deskhaven_core::users::{Password, PasswordError, UserError};
use garde::Validate;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

use crate::audit::{Audit, AuditCtx, Audited};
use crate::auth::{user_agent, with_session};
use crate::me::Me;
use crate::session::{Client, ConsoleUser};
use crate::{AdminApiError, ErrorCode, ValidJson};

/// Longest accepted password input (characters; the policy maximum is 256).
const MAX_PASSWORD_INPUT: usize = 1024;

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
        (status = 409, description = "The account has no local password (`conflict`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong current password (`wrong_password`), or the new one breaks the policy (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn change(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    headers: HeaderMap,
    ValidJson(body): ValidJson<PasswordChange>,
) -> Result<Response, AdminApiError> {
    let user_id = principal.user_id;
    let user = state.users.get(user_id).await.map_err(user_error)?;
    let throttle = &state.login_throttle;
    let attempt = throttle
        .pre_check(client, &user.username)
        .await
        .map_err(crate::auth::throttle_error)?;
    let audit = ctx.entry(Audit::new("user.password_change").target("user", user_id));
    let changed = state
        .users
        .change_password(
            user_id,
            &Password::new(body.current_password.as_str()),
            &Password::new(body.new_password.as_str()),
            &audit,
        )
        .await;
    let stamp = match changed {
        Ok(stamp) => stamp,
        Err(UserError::WrongPassword) => {
            // Counted, like a failed login (a lockout started here is just as final).
            let _ = throttle.on_failure(attempt);
            return Err(AdminApiError::new(
                ErrorCode::WrongPassword,
                "the current password is wrong",
            ));
        }
        Err(err) => {
            if let Err(refund) = throttle.refund(attempt).await {
                tracing::warn!(error = %refund, "cannot refund a password change attempt");
            }
            return Err(user_error(err));
        }
    };
    if let Err(err) = throttle.on_success(attempt).await {
        tracing::warn!(user_id, error = %err, "cannot reset login failure counters");
    }
    let (session, token) = state
        .sessions
        .create_console(user_id, &stamp, Some(client.addr), user_agent(&headers))
        .await
        .map_err(AdminApiError::internal)?;
    let user = state.users.get(user_id).await.map_err(user_error)?;
    let body = (Audited, Json(Me::of(&state, user)));
    Ok(with_session(&session, &token, body).into_response())
}

fn user_error(err: UserError) -> AdminApiError {
    match err {
        UserError::NotFound => AdminApiError::unauthorized(),
        UserError::NotLocal => AdminApiError::conflict("the account has no local password"),
        UserError::PolicyViolation { reason } => {
            AdminApiError::validation_failed("the new password breaks the password policy")
                .with_details(json!({ "fields": { "new_password": [reason.to_string()] } }))
        }
        UserError::Password(PasswordError::Busy) => {
            AdminApiError::unavailable(crate::auth::BUSY_RETRY_AFTER)
        }
        other => AdminApiError::internal(other),
    }
}
