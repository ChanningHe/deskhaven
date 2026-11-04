//! The signed-in user's profile and account standing:
//!
//! - `GET /me/account`: what the account page shows and which actions the account has
//!   (password change: local accounts; TOTP: password accounts, local or LDAP; whether
//!   `auth.tfa.enforce` makes the second factor mandatory).
//! - `PATCH /me`: display name and email. The user is the session's, never a request value;
//!   an address that differs from the stored one (case-insensitively) is unverified again
//!   (core rule), and `email_verified` cannot be sent at all: only an administrator or the
//!   identity provider vouches for an address. Audited as `user.profile_update`.

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
use crate::domain_error::invalid_field;
use crate::me::Me;
use crate::session::ConsoleUser;
use crate::users::view::AuthSourceView;
use crate::{AdminApiError, RoleView, ValidJson};

/// Longest accepted display name and email (characters), as for administrators.
const MAX_NAME_CHARS: usize = 128;
const MAX_EMAIL_CHARS: usize = 254;

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

/// Changes to the user's own profile; absent fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfilePatch {
    /// Blank: the username.
    #[garde(length(chars, max = MAX_NAME_CHARS))]
    pub display_name: Option<String>,
    /// An empty string removes the address. A new address is unverified.
    #[garde(length(chars, max = MAX_EMAIL_CHARS))]
    pub email: Option<String>,
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

/// Changes the signed-in user's display name and email.
#[utoipa::path(
    patch,
    path = "/me",
    operation_id = "update_my_profile",
    tag = "me",
    security(("console_session" = ["self"])),
    request_body = ProfilePatch,
    responses(
        (status = 200, description = "The updated user", body = Me),
        (status = 409, description = "The email belongs to another account (`conflict`)", body = crate::ErrorEnvelope),
        (status = 422, description = "Nothing to change, or an invalid field (`validation_failed`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn edit(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<ProfilePatch>,
) -> Result<(Audited, Json<Me>), AdminApiError> {
    if body.display_name.is_none() && body.email.is_none() {
        return Err(invalid_field("body", "nothing to change"));
    }
    let user_id = principal.user_id;
    let edit = AccountEdit {
        profile: ProfileUpdate {
            display_name: body.display_name,
            email: body.email.map(|e| Some(e).filter(|e| !e.is_empty())),
            ..ProfileUpdate::default()
        },
        ..AccountEdit::default()
    };
    let guard = profile_guard(&state, ctx, user_id);
    let user = state.users.edit(user_id, edit, guard).await?;
    Ok((Audited, Json(Me::of(&state, user))))
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
