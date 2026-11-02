//! `GET /me`: the signed-in console user (self service).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Role, User, UserError};
use serde::Serialize;
use utoipa::ToSchema;

use crate::AdminApiError;
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
}

impl From<User> for Me {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            display_name: u.display_name,
            email: u.email,
            role: u.role.into(),
            must_change_password: u.must_change_password,
            totp_enabled: u.totp_enabled,
        }
    }
}

/// The signed-in console user.
#[utoipa::path(
    get,
    path = "/me",
    tag = "me",
    security(("console_session" = ["self"])),
    responses((status = 200, description = "The signed-in user", body = Me))
)]
pub async fn get_me(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
) -> Result<Json<Me>, AdminApiError> {
    match state.users.get(principal.user_id).await {
        Ok(user) => Ok(Json(user.into())),
        // Deleted since the session was validated.
        Err(UserError::NotFound) => Err(AdminApiError::unauthorized()),
        Err(err) => Err(AdminApiError::internal(err)),
    }
}
