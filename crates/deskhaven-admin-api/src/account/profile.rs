//! The signed-in user's profile and account standing:
//!
//! - `GET /me/account`: what the account page shows and which actions the account has
//!   (password change: local accounts; TOTP: password accounts, local or LDAP; whether
//!   `auth.tfa.enforce` makes the second factor mandatory).
//! - `PATCH /me {display_name}`: the display name only. The user is the session's, never a
//!   request value. The email is not editable here (an administrator sets it): an address
//!   typed by its holder would be unverified yet hold the real owner's address (unique
//!   index), which the OIDC linking refuses to match. Audited as `user.profile_update`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{AccountEdit, AuthSource, ProfileUpdate, User};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::reauth::user_error;
use crate::audit::{Audit, AuditCtx, Audited};
use crate::me::Me;
use crate::session::ConsoleUser;
use crate::users::view::AuthSourceView;
use crate::{AdminApiError, RoleView, ValidJson};

/// Longest accepted display name (characters), as for administrators.
const MAX_NAME_CHARS: usize = 128;

/// The user's own account page. Carries no secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct MyAccount {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub email_verified: bool,
    pub auth_source: AuthSourceView,
    pub role: RoleView,
    pub totp_enabled: bool,
    /// Unused recovery codes (0 while TOTP is off).
    pub recovery_codes_left: u64,
    /// The account has a local password to change (not LDAP or OIDC accounts).
    pub can_change_password: bool,
    /// The account can hold a TOTP second factor (OIDC sign-ins never ask for one).
    pub can_use_totp: bool,
    /// `auth.tfa.enforce` covers this account: TOTP cannot be turned off.
    pub tfa_required: bool,
    pub last_login_at: Option<i64>,
    pub created_at: i64,
}

/// The user's own profile edit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfilePatch {
    /// Blank: the username.
    #[garde(length(chars, max = MAX_NAME_CHARS))]
    pub display_name: String,
}

/// The signed-in user's account standing.
#[utoipa::path(
    get,
    path = "/me/account",
    operation_id = "get_my_account",
    tag = "me",
    security(("console_session" = ["self"])),
    responses((status = 200, description = "The account", body = MyAccount))
)]
pub async fn view(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
) -> Result<Json<MyAccount>, AdminApiError> {
    let user = state
        .users
        .get(principal.user_id)
        .await
        .map_err(user_error)?;
    let tfa = state
        .tfa
        .status(user.id)
        .await
        .map_err(AdminApiError::internal)?;
    let can_use_totp = user.auth_source != AuthSource::Oidc;
    Ok(Json(MyAccount {
        can_change_password: user.auth_source == AuthSource::Local,
        can_use_totp,
        tfa_required: can_use_totp && state.tfa.enforced_for(user.role),
        recovery_codes_left: tfa.recovery_codes_left,
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        email: user.email,
        email_verified: user.email_verified,
        auth_source: user.auth_source.into(),
        role: user.role.into(),
        totp_enabled: user.totp_enabled,
        last_login_at: user.last_login_at.map(|t| t.timestamp_millis()),
        created_at: user.created_at.timestamp_millis(),
    }))
}

/// Changes the signed-in user's display name.
#[utoipa::path(
    patch,
    path = "/me",
    operation_id = "update_my_profile",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = ProfilePatch,
    responses(
        (status = 200, description = "The updated user", body = Me),
        (status = 422, description = "An invalid or unknown field (`validation_failed`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn edit(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<ProfilePatch>,
) -> Result<(Audited, Json<Me>), AdminApiError> {
    let user_id = principal.user_id;
    let current = state.users.get(user_id).await?;
    if !renames(&current, &body.display_name) {
        // Nothing to change: no lock, no write, no audit row (`Audited` also keeps the
        // gate from recording a default one).
        return Ok((Audited, Json(Me::of(&state, current))));
    }
    let edit = AccountEdit {
        profile: ProfileUpdate {
            display_name: Some(body.display_name),
            ..ProfileUpdate::default()
        },
        ..AccountEdit::default()
    };
    let guard = profile_guard(&state, ctx, user_id);
    let user = state.users.edit(user_id, edit, guard).await?;
    Ok((Audited, Json(Me::of(&state, user))))
}

/// Whether `requested` (blank means the username) differs from the stored display name.
fn renames(user: &User, requested: &str) -> bool {
    let requested = requested.trim();
    let wanted = if requested.is_empty() {
        user.username.as_str()
    } else {
        requested
    };
    wanted != user.display_name
}

/// The audited view of the profile fields.
#[derive(Serialize)]
struct ProfileSnapshot {
    display_name: String,
    email: Option<String>,
    email_verified: bool,
}

impl From<&User> for ProfileSnapshot {
    fn from(user: &User) -> Self {
        Self {
            display_name: user.display_name.clone(),
            email: user.email.clone(),
            email_verified: user.email_verified,
        }
    }
}

/// Records `user.profile_update` in the edit's transaction, which also re-checks that the
/// acting account is still active.
fn profile_guard(state: &AppState, ctx: AuditCtx, user_id: i64) -> TxGuard<User> {
    TxGuard::for_account(&state.console_audit, user_id, move |_, before, after| {
        let mut audit = Audit::new("user.profile_update").target("user", user_id);
        if let Some(before) = before {
            audit = audit.before(&ProfileSnapshot::from(before));
        }
        if let Some(after) = after {
            audit = audit.after(&ProfileSnapshot::from(after));
        }
        Ok(ctx.entry(audit))
    })
}
