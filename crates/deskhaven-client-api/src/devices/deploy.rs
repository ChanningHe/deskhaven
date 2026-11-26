//! The client reads only `result` and prints anything else as an error, so every outcome past
//! authentication is a 200 with one.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use deskhaven_core::api_tokens::Scope;
use deskhaven_core::devices::{DeployOutcome, DeployRequest};
use deskhaven_core::state::AppState;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{requester, token_auth};
use crate::ClientApiError;
use crate::client::Client;

/// A real one is about 150 bytes.
pub const MAX_DEPLOY_BODY: usize = 4 * 1024;

#[derive(Deserialize)]
struct Body {
    id: Option<String>,
    uuid: Option<String>,
    pk: Option<String>,
}

pub async fn deploy(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, ClientApiError> {
    let principal = token_auth::authorize(&state, &headers, client, Scope::Deploy).await?;
    // A malformed body is an empty request: INVALID_INPUT, or NOT_ENABLED when off.
    let body: Option<Body> = serde_json::from_slice(&body).ok();
    let (id, uuid, pk) = body.map_or_else(Default::default, |b| (b.id, b.uuid, b.pk));
    let request = DeployRequest {
        rustdesk_id: id.unwrap_or_default(),
        uuid: uuid.unwrap_or_default(),
        pk: pk.unwrap_or_default(),
        by: requester(&principal, client),
    };
    let outcome = state
        .devices
        .deploy(request)
        .await
        .map_err(ClientApiError::internal)?;
    let result = match outcome {
        DeployOutcome::Deployed => "OK",
        DeployOutcome::NotEnabled => "NOT_ENABLED",
        DeployOutcome::InvalidInput => "INVALID_INPUT",
        DeployOutcome::IdTaken => "ID_TAKEN",
        DeployOutcome::TooMany { retry_after } => {
            return Err(ClientApiError::TooManyRequests { retry_after });
        }
        DeployOutcome::Unavailable { retry_after } => {
            return Err(ClientApiError::Unavailable { retry_after });
        }
    };
    Ok(Json(json!({ "result": result })))
}
