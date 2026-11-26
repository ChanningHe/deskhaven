use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use deskhaven_core::addressbooks::{Actor, Tag, client_tag_name};
use deskhaven_core::state::AppState;
use serde::{Deserialize, Serialize};

use super::{ab_error, parse_body, service};
use crate::{BearerUser, ClientApiError};

/// Color is the ARGB int (`Color.value`).
#[derive(Serialize, Deserialize)]
pub struct TagBody {
    name: String,
    color: i64,
}

impl From<Tag> for TagBody {
    fn from(tag: Tag) -> Self {
        Self {
            name: tag.name,
            color: tag.color,
        }
    }
}

impl From<TagBody> for Tag {
    fn from(body: TagBody) -> Self {
        Self {
            name: client_tag_name(&body.name),
            color: body.color,
        }
    }
}

#[derive(Deserialize)]
pub struct RenameBody {
    old: String,
    new: String,
}

/// A JSON list, not a page.
pub async fn list(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
) -> Result<Json<Vec<TagBody>>, ClientApiError> {
    let actor = Actor::User(principal.user_id);
    let tags = service(&state).tags(actor, &guid).await.map_err(ab_error)?;
    Ok(Json(tags.into_iter().map(TagBody::from).collect()))
}

/// An existing one is kept as it is.
pub async fn add(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let tag: TagBody = parse_body(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .add_tag(actor, &guid, tag.into())
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

pub async fn update(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let tag: TagBody = parse_body(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .update_tag(actor, &guid, tag.into())
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

pub async fn rename(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let names: RenameBody = parse_body(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .rename_tag(actor, &guid, &names.old, &client_tag_name(&names.new))
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

pub async fn remove(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let names: Vec<String> = parse_body(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .delete_tags(actor, &guid, &names)
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}
