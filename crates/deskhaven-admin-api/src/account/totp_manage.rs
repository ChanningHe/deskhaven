//! Managing an enabled TOTP second factor (06 §3.3). A stolen session alone must neither
//! remove the factor nor read fresh recovery codes, and neither must a stolen password:
//! both actions need the current password AND a code of the factor (a current TOTP code or
//! an unused recovery code).
//!
//! - `POST /me/totp/disable {current_password, code}`: forgets the seed, the recovery codes
//!   and pending login challenges (`user.totp_disable`), then revokes the user's other
//!   sessions (`session.revoke_others`), like an enrolment rotates them. Refused with 409
//!   `tfa_enforced` while `auth.tfa.enforce` covers the account (checked first, a refused
//!   attempt costs no try, and again against the locked row in the transaction), 409
//!   `conflict` when TOTP is off.
//! - `POST /me/totp/recovery-codes {current_password, code}`: replaces every recovery
//!   code, shown once (`user.recovery_codes_regenerate`; the codes never reach the audit
//!   row).
//!
//! Password and code are one throttle attempt (see `reauth`), so a wrong code counts like a
//! wrong password (422 `wrong_password`, 422 `invalid_code`), and the code is spent
//! (`totp_last_step`, anti-replay) in the transaction of the change: a failure spends
//! nothing. An OIDC account has no password: it sends the code alone, and gets 409
//! `not_local_account` when it holds no TOTP. The account is always the session's.

use std::future::Future;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::console_audit::{Refused, TxGuard};
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::TfaError;
use deskhaven_core::users::{AuthSource, Password, User};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::reauth::{
    Failed, MAX_CODE_INPUT, MAX_PASSWORD_INPUT, not_local, password_failure, throttled, user_error,
};
use super::totp::{no_store, tfa_error};
use crate::audit::{Audit, AuditCtx, Audited};
use crate::domain_error::invalid_field;
use crate::me::Me;
use crate::session::{Client, ConsoleUser};
use crate::transport::SecureTransport;
use crate::{AdminApiError, ErrorCode, ValidJson};

/// [`Refused::code`] of the guard that finds the second factor mandatory for the account.
const TFA_ENFORCED: &str = "tfa_enforced";

/// Proof of possession for a change of the second factor: the password and a code of the
/// factor. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TotpProof {
    /// The account's current password; required except for accounts without a local
    /// password (OIDC), which send the code alone.
    #[garde(length(chars, min = 1, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub current_password: Option<String>,
    /// A current TOTP code, or an unused recovery code.
    #[garde(length(chars, min = 1, max = MAX_CODE_INPUT))]
    pub code: String,
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
    request_body = TotpProof,
    responses(
        (status = 200, description = "Off; the user's other sessions are revoked", body = Me),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 409, description = "Not enabled (`conflict`), required by `auth.tfa.enforce` (`tfa_enforced`), or an OIDC account without TOTP (`not_local_account`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong current password (`wrong_password`), wrong code (`invalid_code`), or a missing password (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated or directory unreachable (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn disable(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<TotpProof>,
) -> Result<(Audited, Json<Me>), AdminApiError> {
    let user_id = principal.user_id;
    let (user, password) = prepare(&state, user_id, &body).await?;
    if is_enforced(&state, &user) {
        return Err(enforced());
    }
    let guard = disable_guard(&state, ctx.clone(), user_id);
    let change = state.tfa.disable_verified(user_id, body.code.trim(), guard);
    reauthenticated(&state, client, &user, password.as_ref(), change).await?;
    revoke_other_sessions(&state, &ctx, principal.session_id, user_id).await;
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
    request_body = TotpProof,
    responses(
        (status = 200, description = "The new codes, shown once", body = RecoveryCodesIssued),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 409, description = "TOTP is not enabled (`conflict`), or an OIDC account without TOTP (`not_local_account`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong current password (`wrong_password`), wrong code (`invalid_code`), or a missing password (`validation_failed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated or directory unreachable (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn regenerate(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<TotpProof>,
) -> Result<Response, AdminApiError> {
    let user_id = principal.user_id;
    let (user, password) = prepare(&state, user_id, &body).await?;
    let audit = ctx.entry(Audit::new("user.recovery_codes_regenerate").target("user", user_id));
    let change = state
        .tfa
        .regenerate_recovery_codes_verified(user_id, body.code.trim(), &audit);
    let codes = reauthenticated(&state, client, &user, password.as_ref(), change).await?;
    let issued = RecoveryCodesIssued {
        recovery_codes: codes.codes().map(str::to_owned).collect(),
    };
    Ok((no_store(), Audited, Json(issued)).into_response())
}

/// The session's account, which must hold a TOTP factor, and the password to check: none
/// for an OIDC account (the code alone proves it), else the one sent.
async fn prepare(
    state: &AppState,
    user_id: i64,
    body: &TotpProof,
) -> Result<(User, Option<Password>), AdminApiError> {
    let user = state.users.get(user_id).await.map_err(user_error)?;
    let oidc = user.auth_source == AuthSource::Oidc;
    if !user.totp_enabled {
        return Err(if oidc {
            not_local()
        } else {
            AdminApiError::conflict("two-factor authentication is not enabled")
        });
    }
    let password = match (oidc, body.current_password.as_deref()) {
        (true, _) => None,
        (false, Some(password)) => Some(Password::new(password)),
        (false, None) => return Err(invalid_field("current_password", "required")),
    };
    Ok((user, password))
}

/// Runs `change` (which spends the code in its transaction) after the password check, as
/// one attempt of the login throttle: a wrong password or code is a counted failure.
async fn reauthenticated<T>(
    state: &AppState,
    client: ClientAddr,
    user: &User,
    password: Option<&Password>,
    change: impl Future<Output = Result<T, TfaError>>,
) -> Result<T, AdminApiError> {
    let verify = async {
        if let Some(password) = password {
            let checked = state.ldap.verify_password(user.id, password).await;
            checked.map_err(password_failure)?;
        }
        change.await.map_err(code_failure)
    };
    throttled(state, client, &user.username, verify).await
}

/// The failure of the changing transaction: a wrong code is a counted failure.
fn code_failure(err: TfaError) -> Failed {
    match err {
        TfaError::InvalidCode => {
            Failed::Counted(AdminApiError::new(ErrorCode::InvalidCode, "wrong code"))
        }
        TfaError::Refused(refused) if refused.code == TFA_ENFORCED => Failed::Refunded(enforced()),
        other => Failed::Refunded(tfa_error(other)),
    }
}

/// `auth.tfa.enforce` makes the factor mandatory for `user` (OIDC sign-ins are exempt).
fn is_enforced(state: &AppState, user: &User) -> bool {
    user.auth_source != AuthSource::Oidc && state.tfa.enforced_for(user.role)
}

fn enforced() -> AdminApiError {
    AdminApiError::new(
        ErrorCode::TfaEnforced,
        "two-factor authentication is required for this account",
    )
}

/// Records `user.totp_disable` in the disabling transaction, which also re-checks that the
/// acting account is still active and that the factor is still not mandatory for the
/// locked row.
fn disable_guard(state: &AppState, ctx: AuditCtx, user_id: i64) -> TxGuard<User> {
    let tfa = state.tfa.clone();
    TxGuard::for_account(
        &state.console_audit,
        user_id,
        move |_, before: Option<&User>, _| {
            let mandatory = before
                .is_some_and(|u| u.auth_source != AuthSource::Oidc && tfa.enforced_for(u.role));
            if mandatory {
                return Err(Refused::new(TFA_ENFORCED));
            }
            Ok(ctx.entry(Audit::new("user.totp_disable").target("user", user_id)))
        },
    )
}

/// The factor changed: sessions that never passed it end, like after an enrolment. The
/// factor is already off, so a failure here is logged, not the request's.
async fn revoke_other_sessions(state: &AppState, ctx: &AuditCtx, keep: i64, user_id: i64) {
    let audit = ctx.entry(Audit::new("session.revoke_others").target("user", user_id));
    if let Err(err) = state
        .sessions
        .revoke_others_audited(user_id, keep, &audit)
        .await
    {
        tracing::warn!(user_id, error = %err, "TOTP disabled, but other sessions could not be revoked");
    }
}
