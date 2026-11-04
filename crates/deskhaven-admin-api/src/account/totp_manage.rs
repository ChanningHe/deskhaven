//! Managing an enabled TOTP second factor (06 §3.3), each action re-authenticated with the
//! current password (see `reauth`): a stolen session alone must neither remove the factor
//! nor read fresh recovery codes.
//!
//! - `POST /me/totp/disable`: forgets the seed, the recovery codes and pending login
//!   challenges (`user.totp_disable`). Refused with 409 `tfa_enforced` while
//!   `auth.tfa.enforce` covers the account (checked first: a refused attempt costs no
//!   password try), 409 `conflict` when TOTP is off, 409 `not_local_account` for OIDC
//!   accounts.
//! - `POST /me/totp/recovery-codes`: replaces every recovery code, shown once
//!   (`user.recovery_codes_regenerate`; the codes never reach the audit row).
//!
//! The account is always the session's. Wrong password: 422 `wrong_password`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{AuthSource, Password, User};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::reauth::{MAX_PASSWORD_INPUT, not_local, user_error, with_current_password};
use super::totp::{no_store, tfa_error};
use crate::audit::{Audit, AuditCtx, Audited};
use crate::me::Me;
use crate::session::{Client, ConsoleUser};
use crate::transport::SecureTransport;
use crate::{AdminApiError, ErrorCode, ValidJson};

/// Re-authentication: the account's current password. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Reauthenticate {
    #[garde(length(chars, min = 1, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub current_password: String,
}

/// Fresh recovery codes, shown once.
// Not `Debug`: it carries the codes.
#[derive(Serialize, ToSchema)]
pub struct RecoveryCodesIssued {
    pub recovery_codes: Vec<String>,
}

/// Turns the user's TOTP off.
#[utoipa::path(
    post,
    path = "/me/totp/disable",
    operation_id = "disable_totp",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = Reauthenticate,
    responses(
        (status = 200, description = "Off", body = Me),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 409, description = "Not enabled (`conflict`), required by `auth.tfa.enforce` (`tfa_enforced`), or an OIDC account (`not_local_account`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong current password (`wrong_password`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn disable(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<Reauthenticate>,
) -> Result<(Audited, Json<Me>), AdminApiError> {
    let user_id = principal.user_id;
    let user = totp_account(&state, user_id).await?;
    if !user.totp_enabled {
        return Err(AdminApiError::conflict(
            "two-factor authentication is not enabled",
        ));
    }
    if state.tfa.enforced_for(user.role) {
        return Err(AdminApiError::new(
            ErrorCode::TfaEnforced,
            "two-factor authentication is required for this account",
        ));
    }
    reauthenticate(&state, client, &user, &body).await?;
    let guard = disable_guard(&state, ctx, user_id);
    state
        .tfa
        .disable_guarded(user_id, guard)
        .await
        .map_err(tfa_error)?;
    let user = state.users.get(user_id).await.map_err(user_error)?;
    Ok((Audited, Json(Me::of(&state, user))))
}

/// Replaces the user's recovery codes.
#[utoipa::path(
    post,
    path = "/me/totp/recovery-codes",
    operation_id = "regenerate_recovery_codes",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = Reauthenticate,
    responses(
        (status = 200, description = "The new codes, shown once", body = RecoveryCodesIssued),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 409, description = "TOTP is not enabled (`conflict`), or an OIDC account (`not_local_account`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong current password (`wrong_password`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn regenerate(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<Reauthenticate>,
) -> Result<Response, AdminApiError> {
    let user_id = principal.user_id;
    let user = totp_account(&state, user_id).await?;
    if !user.totp_enabled {
        return Err(AdminApiError::conflict(
            "two-factor authentication is not enabled",
        ));
    }
    reauthenticate(&state, client, &user, &body).await?;
    let audit = ctx.entry(Audit::new("user.recovery_codes_regenerate").target("user", user_id));
    let codes = state
        .tfa
        .regenerate_recovery_codes_audited(user_id, &audit)
        .await
        .map_err(tfa_error)?;
    let issued = RecoveryCodesIssued {
        recovery_codes: codes.codes().map(str::to_owned).collect(),
    };
    Ok((no_store(), Audited, Json(issued)).into_response())
}

/// The session's account, when it can hold a TOTP factor (OIDC accounts cannot).
async fn totp_account(state: &AppState, user_id: i64) -> Result<User, AdminApiError> {
    let user = state.users.get(user_id).await.map_err(user_error)?;
    if user.auth_source == AuthSource::Oidc {
        return Err(not_local());
    }
    Ok(user)
}

/// The current password of `user` (local hash or directory bind), under the login throttle.
async fn reauthenticate(
    state: &AppState,
    client: deskhaven_core::auth::limiter::ClientAddr,
    user: &User,
    body: &Reauthenticate,
) -> Result<(), AdminApiError> {
    let password = Password::new(body.current_password.as_str());
    let verify = state.ldap.verify_password(user.id, &password);
    with_current_password(state, client, &user.username, verify).await
}

/// Records `user.totp_disable` in the disabling transaction, which also re-checks that the
/// acting account is still active.
fn disable_guard(state: &AppState, ctx: AuditCtx, user_id: i64) -> TxGuard<User> {
    TxGuard::for_account(&state.console_audit, user_id, move |_, _, _| {
        Ok(ctx.entry(Audit::new("user.totp_disable").target("user", user_id)))
    })
}
