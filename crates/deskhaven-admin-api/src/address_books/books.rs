//! `GET|POST /address-books`, `GET|PATCH|DELETE /address-books/{guid}`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::addressbooks::{
    Actor, BookQuery, BookSort, BookSortField, NewSharedBook, SharedBookUpdate,
};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::Deserialize;
use utoipa::IntoParams;

use super::guard::book_guard;
use super::view::{BookPatch, BookView, NewBookBody};
use super::{Problem, service};
use crate::domain_error::invalid_field;
use crate::{
    AdminApiError, AuditCtx, Audited, PageQuery, Paginated, ValidJson, ValidPath, ValidQuery,
};

/// Filters and order of the book list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BookFilter {
    /// `name`, `created_at` or `updated_at`, `-` prefix for descending (default `name`).
    #[garde(length(max = 32))]
    #[param(pattern = "^-?(name|created_at|updated_at)$")]
    pub sort: Option<String>,
    /// Substring of the name (case-insensitive, literal).
    #[garde(length(chars, max = 200))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

fn parse_sort(raw: &str) -> Result<BookSort, AdminApiError> {
    let (name, descending) = match raw.strip_prefix('-') {
        Some(name) => (name, true),
        None => (raw, false),
    };
    let field = match name {
        "name" => BookSortField::Name,
        "created_at" => BookSortField::CreatedAt,
        "updated_at" => BookSortField::UpdatedAt,
        _ => return Err(invalid_field("sort", "unknown sort field")),
    };
    Ok(BookSort { field, descending })
}

/// Shared address books with their owners and the sizes of their parts.
#[utoipa::path(
    get,
    path = "/address-books",
    tag = "address-books",
    operation_id = "list_address_books",
    security(("console_session" = ["admin"])),
    params(PageQuery, BookFilter),
    responses((status = 200, description = "One page of shared address books", body = Paginated<BookView>))
)]
pub async fn list_books(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<BookFilter>,
) -> Result<Json<Paginated<BookView>>, AdminApiError> {
    let query = BookQuery {
        q: filter
            .q
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty()),
        sort: filter.sort.as_deref().map(parse_sort).transpose()?,
        page: page.into(),
    };
    let found = service(&state).admin_page(&query).await?;
    Ok(Json(Paginated::from_page(found, BookView::from)))
}

/// One shared address book.
#[utoipa::path(
    get,
    path = "/address-books/{guid}",
    tag = "address-books",
    operation_id = "get_address_book",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    responses(
        (status = 200, description = "The address book", body = BookView),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn get_book(
    State(state): State<Arc<AppState>>,
    ValidPath(guid): ValidPath<String>,
) -> Result<Json<BookView>, AdminApiError> {
    Ok(Json(service(&state).admin_book(&guid).await?.into()))
}

/// Creates a shared address book (audited `address_book.create`).
#[utoipa::path(
    post,
    path = "/address-books",
    tag = "address-books",
    operation_id = "create_address_book",
    security(("console_session" = ["admin"])),
    request_body = NewBookBody,
    responses(
        (status = 201, description = "Created", body = BookView),
        (status = 404, description = "No such owner", body = Problem),
        (status = 409, description = "Name taken or reserved (`conflict`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn create_book(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewBookBody>,
) -> Result<(StatusCode, Audited, Json<BookView>), AdminApiError> {
    let books = service(&state);
    let new = NewSharedBook {
        name: body.name,
        note: body.note,
        owner_user_id: body.owner_user_id,
    };
    let guard = book_guard(&state, &ctx, "address_book.create", None);
    let created = books.create_shared_guarded(new, guard).await?;
    let summary = books.admin_book(&created.guid).await?;
    Ok((StatusCode::CREATED, Audited, Json(summary.into())))
}

/// Renames a shared address book or changes its note (audited `address_book.update`; an
/// empty patch changes and records nothing).
#[utoipa::path(
    patch,
    path = "/address-books/{guid}",
    tag = "address-books",
    operation_id = "update_address_book",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    request_body = BookPatch,
    responses(
        (status = 200, description = "The updated address book", body = BookView),
        (status = 404, description = "No such shared address book", body = Problem),
        (status = 409, description = "Name taken or reserved (`conflict`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn update_book(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
    ValidJson(body): ValidJson<BookPatch>,
) -> Result<(Audited, Json<BookView>), AdminApiError> {
    let books = service(&state);
    let update = SharedBookUpdate {
        name: body.name,
        note: body.note,
    };
    let guard = book_guard(&state, &ctx, "address_book.update", Some(&guid));
    books
        .update_shared_guarded(Actor::Admin, &guid, update, guard)
        .await?;
    Ok((Audited, Json(books.admin_book(&guid).await?.into())))
}

/// Deletes a shared address book with its entries, tags and grants (audited
/// `address_book.delete`). Clients lose it on their next read.
#[utoipa::path(
    delete,
    path = "/address-books/{guid}",
    tag = "address-books",
    operation_id = "delete_address_book",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn delete_book(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = book_guard(&state, &ctx, "address_book.delete", Some(&guid));
    service(&state)
        .delete_shared_guarded(Actor::Admin, &guid, guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
