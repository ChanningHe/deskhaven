//! Entries of a book: `GET|POST /address-books/{guid}/peers`, `PATCH .../peers/{id}`,
//! `POST .../peers/delete`, `POST .../peers/import`. Passwords go in, never out.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::addressbooks::{Actor, PeerPatch};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::Deserialize;
use utoipa::IntoParams;

use super::guard::book_guard;
use super::view::{IdsBody, ImportDevicesBody, ImportDevicesView, PeerBody, PeerView};
use super::{Problem, service};
use crate::domain_error::invalid_field;
use crate::{
    AdminApiError, AuditCtx, Audited, PageQuery, Paginated, ValidJson, ValidPath, ValidQuery,
};

/// Filter of the entry list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PeerFilter {
    /// Substring of the id, alias, hostname, username or note (case-insensitive, literal).
    #[garde(length(chars, max = 200))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

/// Entries of the book ordered by id.
#[utoipa::path(
    get,
    path = "/address-books/{guid}/peers",
    tag = "address-books",
    operation_id = "list_address_book_peers",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid"), PageQuery, PeerFilter),
    responses(
        (status = 200, description = "One page of entries", body = Paginated<PeerView>),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn list_peers(
    State(state): State<Arc<AppState>>,
    ValidPath(guid): ValidPath<String>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<PeerFilter>,
) -> Result<Json<Paginated<PeerView>>, AdminApiError> {
    let q = filter.q.map(|q| q.trim().to_owned());
    let found = service(&state)
        .peers_matching(Actor::Admin, &guid, q.as_deref(), page.into())
        .await?;
    Ok(Json(Paginated::from_page(found, PeerView::from)))
}

/// Adds an entry (audited `address_book.peer_add`).
#[utoipa::path(
    post,
    path = "/address-books/{guid}/peers",
    tag = "address-books",
    operation_id = "add_address_book_peer",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    request_body = PeerBody,
    responses(
        (status = 201, description = "Added", body = PeerView),
        (status = 404, description = "No such shared address book", body = Problem),
        (status = 409, description = "The id is already in the book, or the book is full (`details.reason = book_full`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn add_peer(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
    ValidJson(body): ValidJson<PeerBody>,
) -> Result<(StatusCode, Audited, Json<PeerView>), AdminApiError> {
    let id = body
        .id
        .clone()
        .ok_or_else(|| invalid_field("id", "is required"))?;
    let patch = PeerPatch::from_json(body.to_json(&id))?;
    let books = service(&state);
    let guard = book_guard(&state, &ctx, "address_book.peer_add", Some(&guid));
    books
        .add_peer_guarded(Actor::Admin, &guid, patch, guard)
        .await?;
    let peer = books.peer(Actor::Admin, &guid, &id).await?;
    Ok((StatusCode::CREATED, Audited, Json(peer.into())))
}

/// Changes an entry; absent fields stay as they are, an empty `password` removes the stored
/// one (audited `address_book.peer_update`).
#[utoipa::path(
    patch,
    path = "/address-books/{guid}/peers/{id}",
    tag = "address-books",
    operation_id = "update_address_book_peer",
    security(("console_session" = ["admin"])),
    params(
        ("guid" = String, Path, description = "Address book guid"),
        ("id" = String, Path, description = "The entry's RustDesk id"),
    ),
    request_body = PeerBody,
    responses(
        (status = 200, description = "The updated entry", body = PeerView),
        (status = 404, description = "No such address book or entry", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn update_peer(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath((guid, id)): ValidPath<(String, String)>,
    ValidJson(body): ValidJson<PeerBody>,
) -> Result<(Audited, Json<PeerView>), AdminApiError> {
    if body.id.as_deref().is_some_and(|given| given != id) {
        return Err(invalid_field("id", "must match the entry in the path"));
    }
    let patch = PeerPatch::from_json(body.to_json(&id))?;
    let books = service(&state);
    let guard = book_guard(&state, &ctx, "address_book.peer_update", Some(&guid));
    books
        .update_peer_guarded(Actor::Admin, &guid, patch, guard)
        .await?;
    let peer = books.peer(Actor::Admin, &guid, &id).await?;
    Ok((Audited, Json(peer.into())))
}

/// Removes entries; ids not in the book are ignored (audited `address_book.peer_delete`).
#[utoipa::path(
    post,
    path = "/address-books/{guid}/peers/delete",
    tag = "address-books",
    operation_id = "delete_address_book_peers",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    request_body = IdsBody,
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn delete_peers(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
    ValidJson(body): ValidJson<IdsBody>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = book_guard(&state, &ctx, "address_book.peer_delete", Some(&guid));
    service(&state)
        .delete_peers_guarded(Actor::Admin, &guid, &body.ids, guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// Adds devices of the inventory as entries: id, name, hostname, user and platform come from
/// the registry; devices the book lists are left as they are (audited
/// `address_book.peer_import`, only when something is added).
#[utoipa::path(
    post,
    path = "/address-books/{guid}/peers/import",
    tag = "address-books",
    operation_id = "import_address_book_peers",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    request_body = ImportDevicesBody,
    responses(
        (status = 200, description = "What was added", body = ImportDevicesView),
        (status = 404, description = "No such shared address book", body = Problem),
        (status = 409, description = "The new entries do not fit (`details.reason = book_full`); nothing is added", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn import_peers(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
    ValidJson(body): ValidJson<ImportDevicesBody>,
) -> Result<(Audited, Json<ImportDevicesView>), AdminApiError> {
    let guard = book_guard(&state, &ctx, "address_book.peer_import", Some(&guid));
    let report = service(&state)
        .import_devices_guarded(Actor::Admin, &guid, &body.device_ids, guard)
        .await?;
    Ok((Audited, Json(report.into())))
}
