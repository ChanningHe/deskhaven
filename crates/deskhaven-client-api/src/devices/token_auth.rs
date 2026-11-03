//! `Authorization: Bearer <API token>` of `--deploy` / `--assign` (03 §1.2). Any problem with
//! the token (unknown, revoked, expired, its creator no longer an active administrator, or
//! lacking the endpoint's scope) is a 401 with the generic message; tokens are never logged.
//!
//! API tokens are not cached, so every request with a token reads the database. While a
//! recent bearer rejection found the address over its per-IP budget
//! (`LoginThrottle::check_bearer`, shared with client sessions), that is a 429 **before** the
//! lookup; each rejected token spends one token of that budget. A missing or malformed header
//! is a plain 401.

use axum::http::HeaderMap;
use deskhaven_core::api_tokens::{ApiTokenPrincipal, Scope};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::state::AppState;

use crate::ClientApiError;
use crate::auth::{bearer_token, too_many, unauthorized};

/// The principal of the request's API token, which must hold `scope`.
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
