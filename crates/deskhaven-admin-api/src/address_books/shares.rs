//! `GET|PUT /address-books/{guid}/shares`, `DELETE /address-books/{guid}/shares/{subject_type}/{subject_id}`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::addressbooks::Actor;
use deskhaven_core::state::AppState;

use super::guard::book_guard;
use super::view::{ItemList, ShareBody, ShareView, SubjectKind, rule_from_number};
use super::{Problem, service};
use crate::domain_error::invalid_field;
use crate::{AdminApiError, AuditCtx, Audited, ValidJson, ValidPath};

/// Who the book is granted to, groups first.
#[utoipa::path(
    get,
    path = "/address-books/{guid}/shares",
    tag = "address-books",
    operation_id = "list_address_book_shares",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    responses(
        (status = 200, description = "The grants", body = ItemList<ShareView>),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn list_shares(
    State(state): State<Arc<AppState>>,
    ValidPath(guid): ValidPath<String>,
) -> Result<Json<ItemList<ShareView>>, AdminApiError> {
    let shares = service(&state).named_shares(Actor::Admin, &guid).await?;
    let items = shares.into_iter().map(ShareView::from).collect();
    Ok(Json(ItemList { items }))
}

/// Grants the book to a user or group, or changes their rule (audited
/// `address_book.share_set`). Rule 1 gives the sharee's client every entry's password.
#[utoipa::path(
    put,
    path = "/address-books/{guid}/shares",
    tag = "address-books",
    operation_id = "set_address_book_share",
    security(("console_session" = ["admin"])),
    params(("guid" = String, Path, description = "Address book guid")),
    request_body = ShareBody,
    responses(
        (status = 204, description = "Granted"),
        (status = 404, description = "No such address book, user or group", body = Problem),
        (status = 422, description = "Invalid field (e.g. the book's owner already has full control)", body = Problem),
    )
)]
pub async fn set_share(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(guid): ValidPath<String>,
    ValidJson(body): ValidJson<ShareBody>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let rule =
        rule_from_number(body.rule).ok_or_else(|| invalid_field("rule", "must be 1, 2 or 3"))?;
    let subject = body.subject_type.subject(body.subject_id);
    let guard = book_guard(&state, &ctx, "address_book.share_set", Some(&guid));
    service(&state)
        .set_share_guarded(Actor::Admin, &guid, subject, rule, guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// Revokes a grant; none is fine (audited `address_book.share_remove`).
#[utoipa::path(
    delete,
    path = "/address-books/{guid}/shares/{subject_type}/{subject_id}",
    tag = "address-books",
    operation_id = "remove_address_book_share",
    security(("console_session" = ["admin"])),
    params(
        ("guid" = String, Path, description = "Address book guid"),
        ("subject_type" = SubjectKind, Path, description = "`user` or `group`"),
        ("subject_id" = i64, Path, description = "User or group id"),
    ),
    responses(
        (status = 204, description = "The subject has no grant"),
        (status = 404, description = "No such shared address book", body = Problem),
    )
)]
pub async fn remove_share(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath((guid, kind, id)): ValidPath<(String, SubjectKind, i64)>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = book_guard(&state, &ctx, "address_book.share_remove", Some(&guid));
    service(&state)
        .remove_share_guarded(Actor::Admin, &guid, kind.subject(id), guard)
        .await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
