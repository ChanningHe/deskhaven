//! `POST /me/identities/{id}/unlink {current_password}`: the account holder removes one of
//! their OIDC identities (M4-09). Re-authenticated with the password like a password change
//! (one throttle attempt, see `reauth`); removing a way in needs no second factor. OIDC
//! accounts have no password (409 `not_local_account`: an administrator unlinks for them),
//! and their last identity stays anyway (409 `last_identity`). Another account's identity
//! is 404. Audited `user.identity_unlink` in the removing transaction. Any removal also marks
//! a verified account address unverified, so the identity is not linked again by email.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::oidc::IdentityChange;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{AuthSource, Password};
use garde::Validate;
use serde::Deserialize;
use serde_json::json;
use utoipa::ToSchema;

use super::reauth::{MAX_PASSWORD_INPUT, not_local, user_error, with_current_password};
use crate::audit::{Audit, AuditCtx, Audited};
use crate::session::{Client, ConsoleUser};
use crate::transport::SecureTransport;
use crate::users::view::IdentityView;
use crate::{AdminApiError, ValidJson, ValidPath};

/// The re-authentication of an unlink. Never logged.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct IdentityUnlink {
    #[garde(length(chars, min = 1, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub current_password: String,
}

/// Removes one of the signed-in user's OIDC identities.
#[utoipa::path(
    post,
    path = "/me/identities/{id}/unlink",
    operation_id = "unlink_my_identity",
    tag = "me",
    security(("console_session" = ["self"])),
    params(("id" = i64, Path, description = "Identity id")),
    request_body = IdentityUnlink,
    responses(
        (status = 204, description = "Unlinked"),
        (status = 403, description = "Plain-HTTP console (`insecure_transport`), or `csrf_failed`", body = crate::ErrorEnvelope),
        (status = 404, description = "No such identity of this account (`not_found`)", body = crate::ErrorEnvelope),
        (status = 409, description = "An OIDC account (`not_local_account`), or its last identity (`last_identity`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Wrong password (`wrong_password`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many attempts (`rate_limited`)", body = crate::ErrorEnvelope),
        (status = 503, description = "Password hashing saturated or directory unreachable (`unavailable`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn unlink(
    State(state): State<Arc<AppState>>,
    _secure: SecureTransport,
    ConsoleUser(principal): ConsoleUser,
    Client(client): Client,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<IdentityUnlink>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let user_id = principal.user_id;
    let user = state.users.get(user_id).await.map_err(user_error)?;
    if user.auth_source == AuthSource::Oidc {
        return Err(not_local());
    }
    let password = Password::new(body.current_password.as_str());
    let check = state.ldap.verify_password(user_id, &password);
    with_current_password(&state, client, &user.username, check).await?;
    let guard = unlink_guard(&state, ctx, user_id);
    state.oidc.unlink_identity(id, Some(user_id), guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// Records `user.identity_unlink` on the holder's own account (still active).
fn unlink_guard(state: &AppState, ctx: AuditCtx, user_id: i64) -> TxGuard<IdentityChange> {
    TxGuard::for_account(
        &state.console_audit,
        user_id,
        move |_, before: Option<&IdentityChange>, _| Ok(ctx.entry(unlink_audit(user_id, before))),
    )
}

/// `user.identity_unlink` on account `user_id`, with the removed identity as `before`.
/// Removing any identity also marks a verified account address unverified (core
/// `unlink_identity`), recorded as `after.email_verified: false` of the same row.
pub(crate) fn unlink_audit(user_id: i64, before: Option<&IdentityChange>) -> Audit {
    let audit = Audit::new("user.identity_unlink").target("user", user_id);
    let Some(change) = before else {
        return audit;
    };
    let audit = audit.before(&IdentityView::from(change.identity.clone()));
    match change.user.email_verified {
        true => audit.after(&json!({ "email_verified": false })),
        false => audit,
    }
}
