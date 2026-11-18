//! `POST /api/oidc/auth`: starts a flow (see the module docs).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::devices::{IdHolder, LIVE_RETRY_AFTER};
use deskhaven_core::oidc::OidcError;
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    ID_TAKEN_MESSAGE, NOT_CONFIGURED_MESSAGE, PROVIDER_UNAVAILABLE_MESSAGE,
    UNKNOWN_PROVIDER_MESSAGE,
};
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
    require_own_id(&state, &device, client).await?;
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

/// 403 [`ID_TAKEN_MESSAGE`] when the id is registered with another uuid: the session would
/// be bound to someone else's device, and a confirmation page naming that id is what its
/// owner expects to see (they share their id for remote support). An id not registered yet
/// passes; the confirmation page says so. 503 when the signal server cannot tell in time.
async fn require_own_id(
    state: &AppState,
    device: &ClientDevice,
    client: ClientAddr,
) -> Result<(), ClientApiError> {
    let holder = state
        .devices
        .id_holder(&device.rustdesk_id, &device.uuid)
        .await
        .map_err(ClientApiError::internal)?;
    match holder {
        IdHolder::ThisDevice | IdHolder::Nobody => Ok(()),
        IdHolder::OtherDevice => {
            let id = device.rustdesk_id.as_str();
            tracing::warn!(rustdesk_id = id, ip = %client.addr, "OIDC login refused: the id belongs to another device");
            Err(ClientApiError::Forbidden(ID_TAKEN_MESSAGE.into()))
        }
        IdHolder::Unknown => Err(ClientApiError::Unavailable {
            retry_after: LIVE_RETRY_AFTER,
        }),
    }
}

fn invalid() -> ClientApiError {
    ClientApiError::BadRequest(INVALID_REQUEST_MESSAGE.into())
}
