//! `GET /me`: the signed-in console user, including a session that must finish its
//! account setup first (`setup_required` says which step).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::sessions::AccountFlags;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Role, User, UserError};
use serde::Serialize;
use utoipa::ToSchema;

use crate::AdminApiError;
use crate::account_setup::{SetupStep, pending_step};
use crate::session::ConsoleUser;

/// Console role (06 §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoleView {
    Owner,
    Admin,
    User,
}

impl From<Role> for RoleView {
    fn from(role: Role) -> Self {
        match role {
            Role::Owner => Self::Owner,
            Role::Admin => Self::Admin,
            Role::User => Self::User,
        }
    }
}

/// The signed-in user's own account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Me {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub email: Option<String>,
    pub role: RoleView,
    pub must_change_password: bool,
    pub totp_enabled: bool,
    /// The account setup step this session must finish before anything else; `null` when
    /// none. Until then only `/me`, the password change, the TOTP enrolment and logout
    /// answer (others: 403 `setup_required`).
    pub setup_required: Option<SetupStep>,
}

impl Me {
    /// `user` as the console shows it, with its pending setup step.
    pub fn of(state: &AppState, user: User) -> Self {
        let setup_required = pending_step(&state.tfa, user.role, AccountFlags::from(&user));
        Self {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            email: user.email,
            role: user.role.into(),
            must_change_password: user.must_change_password,
            totp_enabled: user.totp_enabled,
            setup_required,
        }
    }
}

/// The signed-in console user.
#[utoipa::path(
    get,
    path = "/me",
    tag = "me",
    security(("console_session" = ["session"])),
    responses((status = 200, description = "The signed-in user", body = Me))
)]
pub async fn get_me(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
) -> Result<Json<Me>, AdminApiError> {
    match state.users.get(principal.user_id).await {
        Ok(user) => Ok(Json(Me::of(&state, user))),
        // Deleted since the session was validated.
        Err(UserError::NotFound) => Err(AdminApiError::unauthorized()),
        Err(err) => Err(AdminApiError::internal(err)),
    }
}
