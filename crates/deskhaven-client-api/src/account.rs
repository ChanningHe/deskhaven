//! The `{id, uuid}` body is ignored: the session is already bound to its device, and the
//! client may change its id without logging in again.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Status, UserError};

use crate::auth::unauthorized;
use crate::user_payload::UserPayload;
use crate::{BearerUser, ClientApiError};

/// 401 resets the client's login; any other non-200 is shown as an error.
pub async fn current_user(
    State(state): State<Arc<AppState>>,
    BearerUser(principal): BearerUser,
) -> Result<Json<UserPayload>, ClientApiError> {
    match state.users.get(principal.user_id).await {
        Ok(user) if user.status == Status::Active => Ok(Json(UserPayload::from(&user))),
        // Disabled or unverified while a validated session was cached (a missed event):
        // the fresh read decides.
        Ok(_) => Err(unauthorized()),
        // Deleted after the session was validated.
        Err(UserError::NotFound) => Err(unauthorized()),
        Err(err) => Err(ClientApiError::internal(err)),
    }
}

/// 200 with an empty body; other sessions of the user stay valid.
pub async fn logout(
    State(state): State<Arc<AppState>>,
    BearerUser(principal): BearerUser,
) -> Result<StatusCode, ClientApiError> {
    state
        .sessions
        .revoke(principal.session_id)
        .await
        .map_err(ClientApiError::internal)?;
    Ok(StatusCode::OK)
}
