//! `GET|POST /ldap-directories`, `GET|PATCH|DELETE /ldap-directories/{id}`. Admins read;
//! every write is the owner's (06 §3.6), declared on the route and re-checked on the
//! committed role in the write transaction.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::state::AppState;

use super::guard::directory_guard;
use super::ldap_view::{LdapDirectoryPatch, LdapDirectoryView, NewLdapDirectoryBody};
use crate::{AdminApiError, AuditCtx, Audited, ValidJson, ValidPath};

type Problem = crate::ErrorEnvelope<'static>;

/// Every directory, ordered by name (P0 enables at most one).
#[utoipa::path(
    get,
    path = "/ldap-directories",
    tag = "identity",
    operation_id = "list_ldap_directories",
    security(("console_session" = ["admin"])),
    responses((status = 200, description = "All directories", body = Vec<LdapDirectoryView>))
)]
pub async fn list_ldap_directories(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<LdapDirectoryView>>, AdminApiError> {
    let directories = state.ldap.list_directories().await?;
    Ok(Json(directories.into_iter().map(Into::into).collect()))
}

/// One directory.
#[utoipa::path(
    get,
    path = "/ldap-directories/{id}",
    tag = "identity",
    operation_id = "get_ldap_directory",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Directory id")),
    responses(
        (status = 200, description = "The directory", body = LdapDirectoryView),
        (status = 404, description = "No such directory", body = Problem),
    )
)]
pub async fn get_ldap_directory(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<LdapDirectoryView>, AdminApiError> {
    Ok(Json(state.ldap.get_directory(id).await?.into()))
}

/// Creates a directory (audited `ldap_directory.create`, without the password). Owner only.
#[utoipa::path(
    post,
    path = "/ldap-directories",
    tag = "identity",
    operation_id = "create_ldap_directory",
    security(("console_session" = ["owner"])),
    request_body = NewLdapDirectoryBody,
    responses(
        (status = 201, description = "Created", body = LdapDirectoryView),
        (status = 409, description = "Name taken, or another directory is enabled (`conflict`, `details.field`)", body = Problem),
        (status = 422, description = "Invalid field (e.g. a URL without TLS, a link-local address)", body = Problem),
    )
)]
pub async fn create_ldap_directory(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewLdapDirectoryBody>,
) -> Result<(StatusCode, Audited, Json<LdapDirectoryView>), AdminApiError> {
    let guard = directory_guard(&state, &ctx, "ldap_directory.create");
    let created = state
        .ldap
        .create_directory_guarded(body.into(), guard)
        .await?;
    Ok((StatusCode::CREATED, Audited, Json(created.into())))
}

/// Changes a directory (audited `ldap_directory.update`: url, TLS, bind DN and enabled
/// before and after, never the password). Owner only; directories from the configuration
/// file are read-only.
#[utoipa::path(
    patch,
    path = "/ldap-directories/{id}",
    tag = "identity",
    operation_id = "update_ldap_directory",
    security(("console_session" = ["owner"])),
    params(("id" = i64, Path, description = "Directory id")),
    request_body = LdapDirectoryPatch,
    responses(
        (status = 200, description = "The updated directory", body = LdapDirectoryView),
        (status = 404, description = "No such directory", body = Problem),
        (status = 409, description = "Name taken or another directory enabled (`conflict`), or managed by the configuration file (`managed_by_config`)", body = Problem),
        (status = 422, description = "Invalid field, or the connection changed without `bind_password`", body = Problem),
    )
)]
pub async fn update_ldap_directory(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<LdapDirectoryPatch>,
) -> Result<(Audited, Json<LdapDirectoryView>), AdminApiError> {
    let guard = directory_guard(&state, &ctx, "ldap_directory.update");
    let updated = state
        .ldap
        .update_directory_guarded(id, body.into(), guard)
        .await?;
    Ok((Audited, Json(updated.into())))
}

/// Deletes a directory (audited `ldap_directory.delete`). Its accounts stay, but cannot
/// sign in until a directory is enabled again. Owner only; directories from the
/// configuration file are read-only.
#[utoipa::path(
    delete,
    path = "/ldap-directories/{id}",
    tag = "identity",
    operation_id = "delete_ldap_directory",
    security(("console_session" = ["owner"])),
    params(("id" = i64, Path, description = "Directory id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such directory", body = Problem),
        (status = 409, description = "Managed by the configuration file (`managed_by_config`)", body = Problem),
    )
)]
pub async fn delete_ldap_directory(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = directory_guard(&state, &ctx, "ldap_directory.delete");
    state.ldap.delete_directory_guarded(id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
