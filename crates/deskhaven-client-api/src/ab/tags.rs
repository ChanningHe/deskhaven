//! `POST /api/ab/tags/<guid>`, `POST /api/ab/tag/add/<guid>`,
//! `PUT /api/ab/tag/rename/<guid>`, `PUT /api/ab/tag/update/<guid>`,
//! `DELETE /api/ab/tag/<guid>` (`flutter/lib/models/ab_model.dart:1499-1545, 1776-1877`).

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use deskhaven_core::addressbooks::{Actor, Tag};
use deskhaven_core::state::AppState;
use serde::{Deserialize, Serialize};

use super::{ab_error, parse_body, service};
use crate::{BearerUser, ClientApiError};

/// `{name, color}` both ways; color is the ARGB int (`Color.value`).
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
            name: body.name,
            color: body.color,
        }
    }
}

#[derive(Deserialize)]
pub struct RenameBody {
    old: String,
    new: String,
}

/// Every tag of the book (rule 1): a JSON list, not a page.
pub async fn list(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
) -> Result<Json<Vec<TagBody>>, ClientApiError> {
    let actor = Actor::User(principal.user_id);
    let tags = service(&state).tags(actor, &guid).await.map_err(ab_error)?;
    Ok(Json(tags.into_iter().map(TagBody::from).collect()))
}

/// Adds a tag; an existing one is kept as it is (rule 2).
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

/// Recolours a tag (rule 2).
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

/// Renames a tag, in the peers too (rule 2).
pub async fn rename(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    Path(guid): Path<String>,
    body: Bytes,
) -> Result<StatusCode, ClientApiError> {
    let names: RenameBody = parse_body(&body)?;
    let actor = Actor::User(principal.user_id);
    service(&state)
        .rename_tag(actor, &guid, &names.old, &names.new)
        .await
        .map_err(ab_error)?;
    Ok(StatusCode::OK)
}

/// Deletes the tags the body lists, from the peers too (rule 2).
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
