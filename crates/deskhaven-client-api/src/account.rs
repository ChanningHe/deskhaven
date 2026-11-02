//! `POST /api/currentUser` and `POST /api/logout` (03-client-api-contract §3.3-3.4). Both
//! carry `{id, uuid}`, which is ignored: the session already is bound to the device it was
//! issued to, and the client may change its id without logging in again.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::state::AppState;
use deskhaven_core::users::UserError;

use crate::auth::unauthorized;
use crate::user_payload::UserPayload;
use crate::{BearerUser, ClientApiError};

/// The session user's UserPayload (`flutter/lib/models/user_model.dart:84-100`: 401 resets
/// the login, any other non-200 is shown as an error).
pub async fn current_user(
    State(state): State<Arc<AppState>>,
    BearerUser(principal): BearerUser,
) -> Result<Json<UserPayload>, ClientApiError> {
    match state.users.get(principal.user_id).await {
        Ok(user) => Ok(Json(UserPayload::from(&user))),
        // Deleted after the session was validated.
        Err(UserError::NotFound) => Err(unauthorized()),
        Err(err) => Err(ClientApiError::internal(err)),
    }
}

/// Revokes the presented session; 200 with an empty body (the client ignores the reply,
/// `flutter/lib/models/user_model.dart:170-190`). Other sessions of the user stay valid.
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
