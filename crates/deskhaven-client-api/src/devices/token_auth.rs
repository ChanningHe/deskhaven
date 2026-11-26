//! API tokens are not cached, so every request reads the database; an address over its bearer
//! budget gets a 429 before the lookup.

use axum::http::HeaderMap;
use deskhaven_core::api_tokens::{ApiTokenPrincipal, Scope};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::state::AppState;

use crate::ClientApiError;
use crate::auth::{bearer_token, too_many, unauthorized};

pub(crate) async fn authorize(
    state: &AppState,
    headers: &HeaderMap,
    client: ClientAddr,
    scope: Scope,
) -> Result<ApiTokenPrincipal, ClientApiError> {
    let token = bearer_token(headers).ok_or_else(unauthorized)?;
    state
        .login_throttle
        .check_bearer(client)
        .map_err(too_many)?;
    match state.api_tokens.authenticate(token, scope).await {
        Ok(principal) => Ok(principal),
        Err(err) if err.is_unauthenticated() => {
            tracing::debug!(reason = %err, scope = scope.as_str(), "API token rejected");
            state
                .login_throttle
                .on_bearer_rejected(client)
                .map_err(too_many)?;
            Err(unauthorized())
        }
        Err(err) => Err(ClientApiError::internal(err)),
    }
}
