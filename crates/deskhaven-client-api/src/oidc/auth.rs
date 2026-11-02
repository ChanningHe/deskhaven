//! `POST /api/oidc/auth`: starts a flow (see the module docs).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use deskhaven_core::oidc::OidcError;
use deskhaven_core::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{NOT_CONFIGURED_MESSAGE, PROVIDER_UNAVAILABLE_MESSAGE, UNKNOWN_PROVIDER_MESSAGE};
use crate::ClientApiError;
use crate::client::Client;
use crate::login::request::{INVALID_REQUEST_MESSAGE, device, require_json};

/// Longest accepted `op` (provider names are at most 32 characters).
const MAX_OP_LEN: usize = 64;

/// The body as `OidcSession::auth` sends it (`account.rs:160-176`). Unknown fields ignored.
#[derive(Deserialize)]
struct Request {
    op: Option<String>,
    id: Option<String>,
    uuid: Option<String>,
    #[serde(rename = "deviceInfo")]
    device_info: Option<Value>,
}

/// `OidcAuthUrl` (`account.rs:27-31`): `url` must parse as a URL.
#[derive(Serialize)]
pub struct AuthReply {
    code: String,
    url: String,
}

pub async fn auth(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AuthReply>, ClientApiError> {
    require_json(&headers)?;
    let request: Request = serde_json::from_slice(&body).map_err(|_| invalid())?;
    let op = request
        .op
        .filter(|op| !op.is_empty() && op.len() <= MAX_OP_LEN)
        .ok_or_else(invalid)?;
    let device = device(request.id, request.uuid, request.device_info)?;
    state
        .login_throttle
        .ip()
        .check(client)
        .map_err(|retry_after| ClientApiError::TooManyRequests { retry_after })?;
    let start = state
        .oidc
        .begin_client(&op, &device)
        .await
        .map_err(|err| match err {
            OidcError::NotConfigured => ClientApiError::NotFound(NOT_CONFIGURED_MESSAGE.into()),
            OidcError::UnknownProvider => ClientApiError::NotFound(UNKNOWN_PROVIDER_MESSAGE.into()),
            OidcError::InvalidDevice => invalid(),
            OidcError::Unavailable(_) => {
                ClientApiError::BadGateway(PROVIDER_UNAVAILABLE_MESSAGE.into())
            }
            other => ClientApiError::internal(other),
        })?;
    Ok(Json(AuthReply {
        code: start.code.expose().to_owned(),
        url: start.url.into(),
    }))
}

fn invalid() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_REQUEST_MESSAGE.into())
}
