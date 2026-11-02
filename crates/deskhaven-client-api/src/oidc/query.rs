//! `GET /api/oidc/auth-query`: the client's polling (see the module docs). The session is
//! minted here, bound to the device the flow was started from, under the credential stamp
//! the callback recorded; this is the login event of a successful OIDC login.

use std::sync::Arc;

use axum::Json;
use axum::extract::{RawQuery, State};
use axum::response::{IntoResponse, Response};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::login_events::{LoginMethod, LoginResult};
use deskhaven_core::oidc::{ClientLogin, PollError, PollOutcome};
use deskhaven_core::sessions::SessionError;
use deskhaven_core::state::AppState;

use super::params::param;
use super::{INVALID_LOGIN_MESSAGE, PENDING_MESSAGE, failure_message};
use crate::ClientApiError;
use crate::client::Client;
use crate::login::TokenReply;
use crate::login::event::{self, Attempted};
use crate::login::request::INVALID_REQUEST_MESSAGE;

pub async fn auth_query(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    RawQuery(query): RawQuery,
) -> Result<Response, ClientApiError> {
    let query = query.as_deref();
    let (Some(code), Some(id), Some(uuid)) = (
        param(query, "code"),
        param(query, "id"),
        param(query, "uuid"),
    ) else {
        return Err(invalid_request());
    };
    match state.oidc.poll_client(&code, &id, &uuid).await {
        // 404 with the one message the client keeps polling on.
        Ok(PollOutcome::Pending) => Err(ClientApiError::NotFound(PENDING_MESSAGE.into())),
        Ok(PollOutcome::Failed(failure)) => {
            Err(ClientApiError::Forbidden(failure_message(failure).into()))
        }
        Ok(PollOutcome::Ready(login)) => issue(&state, client, *login).await,
        Err(PollError::Invalid) => Err(invalid_login()),
        Err(err) => Err(ClientApiError::internal(err)),
    }
}

/// The client session and the AuthBody.
async fn issue(
    state: &AppState,
    client: ClientAddr,
    ready: ClientLogin,
) -> Result<Response, ClientApiError> {
    let ClientLogin { login, device } = ready;
    let created = state
        .sessions
        .create_client(login.user.id, &login.stamp, &device, Some(client.addr))
        .await;
    let attempted = Attempted {
        username: &login.user.username,
        rustdesk_id: &device.rustdesk_id,
        method: LoginMethod::Oidc,
    };
    match created {
        Ok((_, token)) => {
            event::record(
                state,
                client,
                &attempted,
                Some(login.user.id),
                LoginResult::Success,
            )
            .await;
            Ok(Json(TokenReply::new(&login.user, &token)).into_response())
        }
        // Disabled, deleted, password or auth source changed since the callback.
        Err(SessionError::StaleCredential) => {
            let result = LoginResult::InvalidCredentials;
            event::record(state, client, &attempted, Some(login.user.id), result).await;
            Err(invalid_login())
        }
        Err(err) => Err(ClientApiError::internal(err)),
    }
}

fn invalid_login() -> ClientApiError {
    ClientApiError::Unauthorized(INVALID_LOGIN_MESSAGE.into())
}

fn invalid_request() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_REQUEST_MESSAGE.into())
}
