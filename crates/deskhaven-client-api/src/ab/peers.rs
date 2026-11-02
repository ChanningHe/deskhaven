//! `POST /api/ab/peers`, `POST /api/ab/peer/add/<guid>`, `PUT /api/ab/peer/update/<guid>`,
//! `DELETE /api/ab/peer/<guid>`. The peer JSON codec (relay string, `hash` / `password`,
//! `extra`) is `deskhaven_core::addressbooks::PeerPatch` / `Peer::to_json`.

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use deskhaven_core::addressbooks::{Actor, PeerPatch};
use deskhaven_core::state::AppState;
use serde_json::{Map, Value};

use super::books::PageReply;
use super::{ab_error, parse_body, query, service};
use crate::{BearerUser, ClientApiError};

/// One page of a book's peers (rule 1).
pub async fn list(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<PageReply<Map<String, Value>>>, ClientApiError> {
    let page = query::page(raw.as_deref())?;
    let guid = query::book(raw.as_deref())?;
    let actor = Actor::User(principal.user_id);
    let peers = service(&state)
        .peers(actor, guid, page)
        .await
        .map_err(ab_error)?;
    Ok(Json(PageReply {
        total: peers.total,
        data: peers.items.iter().map(|peer| peer.to_json()).collect(),
    }))
}

/// Adds one peer (rule 2).
pub async fn add(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let patch = peer_patch(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .add_peer(actor, &guid, patch)
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

/// Changes the fields of one peer that the body carries (rule 2).
pub async fn update(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let patch = peer_patch(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .update_peer(actor, &guid, patch)
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

/// Deletes the peers whose ids the body lists (rule 2).
pub async fn remove(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let ids: Vec<String> = parse_body(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .delete_peers(actor, &guid, &ids)
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

fn peer_patch(body: &[u8]) -> Result<PeerPatch, ClientApiError> {
    let value: Value = parse_body(body)?;
    PeerPatch::from_json(value).map_err(ab_error)
}
