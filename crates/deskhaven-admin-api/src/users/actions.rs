//! Account actions: `POST /users/{id}/{disable,enable,reset-password,reset-2fa,
//! revoke-sessions,unlock}`. Each is one guarded, audited transaction.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::auth::limiter::{Unlocked, unlock_guarded};
use deskhaven_core::console_audit::{GuardError, TxGuard};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{AccountEdit, Status, User};

use super::rules::{Acting, extra};
use super::view::{LockoutCleared, SessionsRevoked, TemporaryPassword, UserView};
use crate::domain_error::refused;
use crate::{AdminApiError, Audit, AuditCtx, Audited, ConsoleUser, ValidPath};

type Problem = crate::ErrorEnvelope<'static>;

async fn set_status(
    state: &AppState,
    acting: &Acting,
    id: i64,
    status: Status,
    action: &'static str,
) -> Result<UserView, AdminApiError> {
    let edit = AccountEdit {
        status: Some(status),
        ..AccountEdit::default()
    };
    let guard = acting.user_guard(&state.console_audit, action, None);
    Ok(state.users.edit(id, edit, guard).await?.into())
}

/// Disables the account and revokes all its sessions (audited `user.disable`).
#[utoipa::path(
    post,
    path = "/users/{id}/disable",
    tag = "users",
    operation_id = "disable_user",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 200, description = "The disabled account", body = UserView),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
        (status = 409, description = "The last owner (`last_owner`)", body = Problem),
    )
)]
pub async fn disable_user(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, Json<UserView>), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let user = set_status(&state, &acting, id, Status::Disabled, "user.disable").await?;
    Ok((Audited, Json(user)))
}

/// Activates the account (audited `user.enable`). Old sessions stay revoked.
#[utoipa::path(
    post,
    path = "/users/{id}/enable",
    tag = "users",
    operation_id = "enable_user",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 200, description = "The active account", body = UserView),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
    )
)]
pub async fn enable_user(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, Json<UserView>), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let user = set_status(&state, &acting, id, Status::Active, "user.enable").await?;
    Ok((Audited, Json(user)))
}

/// Replaces the local password with a generated temporary one (returned once), requires a
/// change at next login and revokes every session (audited `user.reset_password`; the
/// password is not in the audit row).
#[utoipa::path(
    post,
    path = "/users/{id}/reset-password",
    tag = "users",
    operation_id = "reset_user_password",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 200, description = "The temporary password, shown only here", body = TemporaryPassword),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
        (status = 409, description = "LDAP / OIDC account (`not_local_account`)", body = Problem),
    )
)]
pub async fn reset_password(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, Json<TemporaryPassword>), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let password = state.users.temporary_password()?;
    let audit_extra = extra("password", "reset");
    let guard = acting.user_guard(&state.console_audit, "user.reset_password", audit_extra);
    state
        .users
        .set_password_guarded(id, &password, true, guard)
        .await?;
    let shown = TemporaryPassword {
        temporary_password: password.expose().to_owned(),
    };
    Ok((Audited, Json(shown)))
}

/// Turns two-factor authentication off: seed, recovery codes and pending challenges are
/// deleted; the user can enrol again (audited `user.reset_2fa`).
#[utoipa::path(
    post,
    path = "/users/{id}/reset-2fa",
    tag = "users",
    operation_id = "reset_user_2fa",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 204, description = "2FA is off"),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
    )
)]
pub async fn reset_2fa(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let guard = acting.user_guard(&state.console_audit, "user.reset_2fa", None);
    state.tfa.disable_guarded(id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// Revokes every client and console session of the account; its clients must log in
/// again (audited `user.revoke_sessions`).
#[utoipa::path(
    post,
    path = "/users/{id}/revoke-sessions",
    tag = "users",
    operation_id = "revoke_user_sessions",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 200, description = "How many sessions were revoked", body = SessionsRevoked),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
    )
)]
pub async fn revoke_sessions(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, Json<SessionsRevoked>), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let audit_extra = extra("sessions", "revoked");
    let guard = acting.user_guard(&state.console_audit, "user.revoke_sessions", audit_extra);
    let revoked = state
        .sessions
        .revoke_all_for_user_guarded(id, guard)
        .await?;
    Ok((Audited, Json(SessionsRevoked { revoked })))
}

/// Clears the account's login lockout and failure counters (audited `user.unlock`).
///
/// The owner rule is checked against the account as read just before the unlock (the
/// lockout table is keyed by username and does not lock the account row); an unlock only
/// removes a brute-force brake, it grants nothing.
#[utoipa::path(
    post,
    path = "/users/{id}/unlock",
    tag = "users",
    operation_id = "unlock_user",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "User id")),
    responses(
        (status = 200, description = "Counters removed", body = LockoutCleared),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such user", body = Problem),
    )
)]
pub async fn unlock(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, Json<LockoutCleared>), AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let user = state.users.get(id).await?;
    acting.may_manage(&user).map_err(refused)?;
    let guard = unlock_guard(&state, &acting, &user);
    let cleared = unlock_guarded(&state.db, &user.username, guard)
        .await
        .map_err(|err| match err {
            GuardError::Refused(r) => refused(r),
            GuardError::Database(db) => AdminApiError::internal(db),
        })?;
    Ok((Audited, Json(LockoutCleared { cleared })))
}

fn unlock_guard(state: &AppState, acting: &Acting, user: &User) -> TxGuard<Unlocked> {
    let (ctx, id) = (acting.ctx.clone(), user.id);
    TxGuard::new(&state.console_audit, move |_, after: Option<&Unlocked>| {
        let cleared = after.map_or(0, |u| u.cleared);
        let audit = Audit::new("user.unlock")
            .target("user", id)
            .after(&serde_json::json!({ "lockout_entries_cleared": cleared }));
        Ok(ctx.entry(audit))
    })
}
