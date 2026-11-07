//! `DELETE /users/{id}/identities/{identity_id}`: an administrator removes an account's
//! OIDC identity (M4-09; the identities are listed in `GET /users/{id}`). Administrators
//! view and unlink only, they never link for someone. The account rules apply (only an
//! owner touches an owner's account), checked on the committed state in the removing
//! transaction, which records `user.identity_unlink`. The last identity of an OIDC account
//! stays (409 `last_identity`).

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::oidc::IdentityChange;
use deskhaven_core::state::AppState;

use super::rules::check_target;
use crate::account::unlink_audit;
use crate::{AdminApiError, AuditCtx, Audited, ValidPath};

type Problem = crate::ErrorEnvelope<'static>;

/// Removes an OIDC identity of an account.
#[utoipa::path(
    delete,
    path = "/users/{id}/identities/{identity_id}",
    tag = "users",
    operation_id = "unlink_user_identity",
    security(("console_session" = ["admin"])),
    params(
        ("id" = i64, Path, description = "User id"),
        ("identity_id" = i64, Path, description = "Identity id"),
    ),
    responses(
        (status = 204, description = "Unlinked"),
        (status = 403, description = "Owner accounts need an owner (`owner_required`)", body = Problem),
        (status = 404, description = "No such identity of this user", body = Problem),
        (status = 409, description = "The last identity of an OIDC account (`last_identity`)", body = Problem),
    )
)]
pub async fn unlink_user_identity(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath((id, identity_id)): ValidPath<(i64, i64)>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = admin_unlink_guard(&state, ctx);
    state
        .oidc
        .unlink_identity(identity_id, Some(id), guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

fn admin_unlink_guard(state: &AppState, ctx: AuditCtx) -> TxGuard<IdentityChange> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |role, before: Option<&IdentityChange>, _| {
            check_target(role, before.map(|c| &c.user), None)?;
            let user_id = before.map_or(0, |c| c.user.id);
            Ok(entry_ctx.entry(unlink_audit(user_id, before)))
        },
    )
}
