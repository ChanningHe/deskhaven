//! Tags of a book: `GET|POST /address-books/{guid}/tags`, `PATCH .../tags/{name}`,
//! `POST .../tags/delete`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::addressbooks::{Actor, DEFAULT_TAG_COLOR, Tag};
use deskhaven_core::state::AppState;

use super::guard::book_guard;
use super::view::{ItemList, NamesBody, NewTagBody, TagPatch, TagView};
use super::{Problem, service};
use crate::domain_error::invalid_field;
use crate::{AdminApiError, AuditCtx, Audited, ValidJson, ValidPath};

/// The tags of the book, ordered by name.
#[utoipa::path(
    get,
    path = "/address-books/{guid}/tags",
    tag = "address-books",
    operation_id = "list_address_book_tags",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    responses(
        (status = 200, description = "The tags", body = ItemList<TagView>),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn list_tags(
    State(state): State<Arc<AppState>>,
    ValidPath(guid): ValidPath<String>,
) -> Result<Json<ItemList<TagView>>, AdminApiError> {
    let tags = service(&state).tags(Actor::Admin, &guid).await?;
    let items = tags.into_iter().map(TagView::from).collect();
    Ok(Json(ItemList { items }))
}

/// Adds a tag (audited `address_book.tag_add`).
#[utoipa::path(
    post,
    path = "/address-books/{guid}/tags",
    tag = "address-books",
    operation_id = "add_address_book_tag",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    request_body = NewTagBody,
    responses(
        (status = 201, description = "Added", body = TagView),
        (status = 404, description = "No such shared address book", body = Problem),
        (status = 409, description = "The book has a tag of that name", body = Problem),
        (status = 422, description = "Invalid field, or the book holds the most tags it may", body = Problem),
    )
)]
pub async fn add_tag(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
    ValidJson(body): ValidJson<NewTagBody>,
) -> Result<(StatusCode, Audited, Json<TagView>), AdminApiError> {
    let books = service(&state);
    if books
        .tags(Actor::Admin, &guid)
        .await?
        .iter()
        .any(|t| t.name == body.name)
    {
        return Err(deskhaven_core::addressbooks::AddressBookError::TagExists.into());
    }
    let tag = Tag {
        name: body.name,
        color: body.color.unwrap_or(DEFAULT_TAG_COLOR),
    };
    let guard = book_guard(&state, &ctx, "address_book.tag_add", Some(&guid));
    books
        .add_tag_guarded(Actor::Admin, &guid, tag.clone(), guard)
        .await?;
    Ok((StatusCode::CREATED, Audited, Json(tag.into())))
}

/// Renames and / or recolours a tag (audited `address_book.tag_rename` / `tag_update`).
/// Renaming also renames it on the entries. With both, the rename comes first.
#[utoipa::path(
    patch,
    path = "/address-books/{guid}/tags/{name}",
    tag = "address-books",
    operation_id = "update_address_book_tag",
    security(("console_session" = ["admin"])),
    params(
        ("guid" = String, Path, description = "Address book guid"),
        ("name" = String, Path, description = "The tag's name"),
    ),
    request_body = TagPatch,
    responses(
        (status = 200, description = "The updated tag", body = TagView),
        (status = 404, description = "No such address book or tag", body = Problem),
        (status = 409, description = "The book has a tag of the new name", body = Problem),
        (status = 422, description = "Invalid field, or neither name nor color given", body = Problem),
    )
)]
pub async fn update_tag(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath((guid, name)): ValidPath<(String, String)>,
    ValidJson(body): ValidJson<TagPatch>,
) -> Result<(Audited, Json<TagView>), AdminApiError> {
    if body.name.is_none() && body.color.is_none() {
        return Err(invalid_field("name", "give a new name, a color or both"));
    }
    let books = service(&state);
    let mut current = name.clone();
    if let Some(new) = body.name.as_deref().filter(|new| *new != name) {
        let guard = book_guard(&state, &ctx, "address_book.tag_rename", Some(&guid));
        books
            .rename_tag_guarded(Actor::Admin, &guid, &name, new, guard)
            .await?;
        new.clone_into(&mut current);
    }
    if let Some(color) = body.color {
        let guard = book_guard(&state, &ctx, "address_book.tag_update", Some(&guid));
        let tag = Tag {
            name: current.clone(),
            color,
        };
        books
            .update_tag_guarded(Actor::Admin, &guid, tag, guard)
            .await?;
    }
    let tags = books.tags(Actor::Admin, &guid).await?;
    let found = tags.into_iter().find(|t| t.name == current);
    let tag = found.ok_or_else(|| AdminApiError::not_found("tag not found"))?;
    Ok((Audited, Json(tag.into())))
}

/// Deletes tags from the book and from its entries; unknown names are ignored (audited
/// `address_book.tag_delete`).
#[utoipa::path(
    post,
    path = "/address-books/{guid}/tags/delete",
    tag = "address-books",
    operation_id = "delete_address_book_tags",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    request_body = NamesBody,
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn delete_tags(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
    ValidJson(body): ValidJson<NamesBody>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = book_guard(&state, &ctx, "address_book.tag_delete", Some(&guid));
    service(&state)
        .delete_tags_guarded(Actor::Admin, &guid, &body.names, guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
