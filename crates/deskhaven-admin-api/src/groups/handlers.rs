//! `GET|POST /groups`, `GET|PATCH|DELETE /groups/{id}`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::groups::{Group, GroupQuery, GroupSort, GroupSortField, GroupUpdate, NewGroup};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;

use super::guard::group_guard;
use super::view::{GroupPatch, GroupView, NewGroupBody};
use crate::domain_error::invalid_field;
use crate::{
    AdminApiError, AuditCtx, Audited, PageQuery, Paginated, ValidJson, ValidPath, ValidQuery,
};

type Problem = crate::ErrorEnvelope<'static>;

/// Filters and order of the group list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GroupFilter {
    /// `name` or `created_at`, `-` prefix for descending (default `name`).
    #[garde(length(max = 32))]
    #[param(pattern = "^-?(name|created_at)$")]
    pub sort: Option<String>,
    /// Substring of the name (case-insensitive, literal).
    #[garde(length(chars, max = 200))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

fn parse_sort(raw: &str) -> Result<GroupSort, AdminApiError> {
    let (name, descending) = match raw.strip_prefix('-') {
        Some(name) => (name, true),
        None => (raw, false),
    };
    let field = match name {
        "name" => GroupSortField::Name,
        "created_at" => GroupSortField::CreatedAt,
        _ => return Err(invalid_field("sort", "unknown sort field")),
    };
    Ok(GroupSort { field, descending })
}

/// User groups with member counts.
#[utoipa::path(
    get,
    path = "/groups",
    tag = "groups",
    operation_id = "list_groups",
    security(("console_session" = ["admin"])),
    params(PageQuery, GroupFilter),
    responses((status = 200, description = "One page of groups", body = Paginated<GroupView>))
)]
pub async fn list_groups(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<GroupFilter>,
) -> Result<Json<Paginated<GroupView>>, AdminApiError> {
    let query = GroupQuery {
        q: filter
            .q
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty()),
        sort: filter.sort.as_deref().map(parse_sort).transpose()?,
        page: page.into(),
    };
    let found = state.groups.page(&query).await?;
    Ok(Json(Paginated::from_page(found, GroupView::from)))
}

/// One group.
#[utoipa::path(
    get,
    path = "/groups/{id}",
    tag = "groups",
    operation_id = "get_group",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Group id")),
    responses(
        (status = 200, description = "The group", body = GroupView),
        (status = 404, description = "No such group", body = Problem),
    )
)]
pub async fn get_group(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<GroupView>, AdminApiError> {
    Ok(Json(state.groups.summary(id).await?.into()))
}

/// Creates a group (audited `group.create`).
#[utoipa::path(
    post,
    path = "/groups",
    tag = "groups",
    operation_id = "create_group",
    security(("console_session" = ["admin"])),
    request_body = NewGroupBody,
    responses(
        (status = 201, description = "Created", body = GroupView),
        (status = 409, description = "Name taken (`conflict`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn create_group(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewGroupBody>,
) -> Result<(StatusCode, Audited, Json<GroupView>), AdminApiError> {
    let new = NewGroup {
        name: body.name,
        note: body.note,
    };
    let guard = group_guard(&state, &ctx, "group.create");
    let group = state.groups.create_guarded(new, guard).await?;
    Ok((StatusCode::CREATED, Audited, Json(GroupView::new(group, 0))))
}

/// Renames a group or changes its note (audited `group.update`). Groups from the
/// configuration file are read-only.
#[utoipa::path(
    patch,
    path = "/groups/{id}",
    tag = "groups",
    operation_id = "update_group",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Group id")),
    request_body = GroupPatch,
    responses(
        (status = 200, description = "The updated group", body = GroupView),
        (status = 404, description = "No such group", body = Problem),
        (status = 409, description = "Name taken (`conflict`) or managed by the configuration file (`managed_by_config`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn update_group(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<GroupPatch>,
) -> Result<(Audited, Json<GroupView>), AdminApiError> {
    let update = GroupUpdate {
        name: body.name,
        note: body.note,
    };
    let guard = group_guard(&state, &ctx, "group.update");
    let group: Group = state.groups.update_guarded(id, update, guard).await?;
    let summary = state.groups.summary(group.id).await?;
    Ok((Audited, Json(summary.into())))
}

/// Deletes a group, its memberships, its address book shares and strategy assignment
/// (audited `group.delete`). Groups from the configuration file are read-only.
#[utoipa::path(
    delete,
    path = "/groups/{id}",
    tag = "groups",
    operation_id = "delete_group",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Group id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such group", body = Problem),
        (status = 409, description = "Managed by the configuration file (`managed_by_config`)", body = Problem),
    )
)]
pub async fn delete_group(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = group_guard(&state, &ctx, "group.delete");
    state.groups.delete_guarded(id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// Snapshot of a group in its audit rows.
#[derive(Serialize)]
pub(crate) struct GroupSnapshot<'a> {
    name: &'a str,
    note: &'a str,
}

impl<'a> From<&'a Group> for GroupSnapshot<'a> {
    fn from(g: &'a Group) -> Self {
        Self {
            name: &g.name,
            note: &g.note,
        }
    }
}
