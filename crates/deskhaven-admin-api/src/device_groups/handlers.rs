//! `GET|POST /device-groups`, `GET|PATCH|DELETE /device-groups/{id}`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::devices::{
    DeviceGroup, DeviceGroupQuery, DeviceGroupSort, DeviceGroupSortField, DeviceGroupUpdate,
};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;

use super::view::{DeviceGroupPatch, DeviceGroupView, NewDeviceGroupBody};
use crate::domain_error::invalid_field;
use crate::{
    AdminApiError, Audit, AuditCtx, Audited, PageQuery, Paginated, ValidJson, ValidPath, ValidQuery,
};

type Problem = crate::ErrorEnvelope<'static>;

/// Filters and order of the device group list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeviceGroupFilter {
    /// `name` or `created_at`, `-` prefix for descending (default `name`).
    #[garde(length(max = 32))]
    #[param(pattern = "^-?(name|created_at)$")]
    pub sort: Option<String>,
    /// Substring of the name (case-insensitive, literal).
    #[garde(length(chars, max = 200))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

fn parse_sort(raw: &str) -> Result<DeviceGroupSort, AdminApiError> {
    let (name, descending) = match raw.strip_prefix('-') {
        Some(name) => (name, true),
        None => (raw, false),
    };
    let field = match name {
        "name" => DeviceGroupSortField::Name,
        "created_at" => DeviceGroupSortField::CreatedAt,
        _ => return Err(invalid_field("sort", "unknown sort field")),
    };
    Ok(DeviceGroupSort { field, descending })
}

/// Snapshot of a device group in its audit rows.
#[derive(Serialize)]
struct GroupSnapshot<'a> {
    name: &'a str,
    note: &'a str,
}

/// Any admin may manage device groups: beyond re-checking that the actor is still an admin
/// or owner the guard never refuses.
fn group_guard(state: &AppState, ctx: &AuditCtx, action: &'static str) -> TxGuard<DeviceGroup> {
    let entry_ctx = ctx.clone();
    ctx.admin_guard(
        &state.console_audit,
        move |_, before: Option<&DeviceGroup>, after: Option<&DeviceGroup>| {
            let id = after.or(before).map_or(0, |g| g.id);
            let mut audit = Audit::new(action).target("device_group", id);
            if let Some(g) = before {
                audit = audit.before(&GroupSnapshot {
                    name: &g.name,
                    note: &g.note,
                });
            }
            if let Some(g) = after {
                audit = audit.after(&GroupSnapshot {
                    name: &g.name,
                    note: &g.note,
                });
            }
            Ok(entry_ctx.entry(audit))
        },
    )
}

/// Device groups with their device counts.
#[utoipa::path(
    get,
    path = "/device-groups",
    tag = "device-groups",
    operation_id = "list_device_groups",
    security(("console_session" = ["admin"])),
    params(PageQuery, DeviceGroupFilter),
    responses((status = 200, description = "One page of device groups", body = Paginated<DeviceGroupView>))
)]
pub async fn list_device_groups(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(filter): ValidQuery<DeviceGroupFilter>,
) -> Result<Json<Paginated<DeviceGroupView>>, AdminApiError> {
    let query = DeviceGroupQuery {
        q: filter
            .q
            .map(|q| q.trim().to_owned())
            .filter(|q| !q.is_empty()),
        sort: filter.sort.as_deref().map(parse_sort).transpose()?,
        page: page.into(),
    };
    let found = state.devices.group_page(&query).await?;
    Ok(Json(Paginated::from_page(found, DeviceGroupView::from)))
}

/// One device group.
#[utoipa::path(
    get,
    path = "/device-groups/{id}",
    tag = "device-groups",
    operation_id = "get_device_group",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Device group id")),
    responses(
        (status = 200, description = "The device group", body = DeviceGroupView),
        (status = 404, description = "No such device group", body = Problem),
    )
)]
pub async fn get_device_group(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<DeviceGroupView>, AdminApiError> {
    Ok(Json(state.devices.group_summary(id).await?.into()))
}

/// Creates a device group (audited `device_group.create`).
#[utoipa::path(
    post,
    path = "/device-groups",
    tag = "device-groups",
    operation_id = "create_device_group",
    security(("console_session" = ["admin"])),
    request_body = NewDeviceGroupBody,
    responses(
        (status = 201, description = "Created", body = DeviceGroupView),
        (status = 409, description = "Name taken (`conflict`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn create_device_group(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<NewDeviceGroupBody>,
) -> Result<(StatusCode, Audited, Json<DeviceGroupView>), AdminApiError> {
    let guard = group_guard(&state, &ctx, "device_group.create");
    let created = state
        .devices
        .create_group_guarded(&body.name, &body.note, guard)
        .await?;
    let summary = state.devices.group_summary(created.id).await?;
    Ok((StatusCode::CREATED, Audited, Json(summary.into())))
}

/// Renames a device group or changes its note (audited `device_group.update`). Groups from
/// the configuration file are read-only.
#[utoipa::path(
    patch,
    path = "/device-groups/{id}",
    tag = "device-groups",
    operation_id = "update_device_group",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Device group id")),
    request_body = DeviceGroupPatch,
    responses(
        (status = 200, description = "The updated device group", body = DeviceGroupView),
        (status = 404, description = "No such device group", body = Problem),
        (status = 409, description = "Name taken (`conflict`) or managed by the configuration file (`managed_by_config`)", body = Problem),
        (status = 422, description = "Invalid field", body = Problem),
    )
)]
pub async fn update_device_group(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<DeviceGroupPatch>,
) -> Result<(Audited, Json<DeviceGroupView>), AdminApiError> {
    let update = DeviceGroupUpdate {
        name: body.name,
        note: body.note,
    };
    let guard = group_guard(&state, &ctx, "device_group.update");
    state
        .devices
        .update_group_guarded(id, update, guard)
        .await?;
    let summary = state.devices.group_summary(id).await?;
    Ok((Audited, Json(summary.into())))
}

/// Deletes a device group and its strategy assignment; its devices become ungrouped
/// (audited `device_group.delete`). Groups from the configuration file are read-only.
#[utoipa::path(
    delete,
    path = "/device-groups/{id}",
    tag = "device-groups",
    operation_id = "delete_device_group",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Device group id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such device group", body = Problem),
        (status = 409, description = "Managed by the configuration file (`managed_by_config`)", body = Problem),
    )
)]
pub async fn delete_device_group(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = group_guard(&state, &ctx, "device_group.delete");
    state.devices.delete_group_guarded(id, guard).await?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
