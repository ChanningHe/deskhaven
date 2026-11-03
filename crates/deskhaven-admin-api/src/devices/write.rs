//! `PATCH|DELETE /devices/{id}`, `POST /devices/{id}/disconnect`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use deskhaven_core::devices::{DISCONNECT_TTL, DeviceStatus, DeviceUpdate};
use deskhaven_core::state::AppState;

use super::guard::{device_guard, disconnect_guard};
use super::view::{DevicePatch, DeviceView, DisconnectBody, DisconnectQueued};
use crate::{AdminApiError, AuditCtx, Audited, ValidJson, ValidPath};

type Problem = crate::ErrorEnvelope<'static>;

/// Assigns the owner and device group, changes the note, disables or enables the device
/// (audited `device.update`; a patch that only sets the status is `device.disable` or
/// `device.enable`).
#[utoipa::path(
    patch,
    path = "/devices/{id}",
    tag = "devices",
    operation_id = "update_device",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Device id")),
    request_body = DevicePatch,
    responses(
        (status = 200, description = "The updated device", body = DeviceView),
        (status = 404, description = "No such device", body = Problem),
        (status = 422, description = "Invalid field, or an owner or device group that does not exist", body = Problem),
    )
)]
pub async fn update_device(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<DevicePatch>,
) -> Result<(Audited, Json<DeviceView>), AdminApiError> {
    let update = DeviceUpdate::from(body);
    let guard = device_guard(&state, &ctx, action_of(&update), id);
    state.devices.update_guarded(id, update, guard).await?;
    let summary = state.devices.summary(id).await?;
    Ok((Audited, Json(summary.into())))
}

/// The audit action of an update: a lone status change reads as disable / enable.
fn action_of(update: &DeviceUpdate) -> &'static str {
    let only_status =
        update.owner_user_id.is_none() && update.device_group_id.is_none() && update.note.is_none();
    match (only_status, update.status) {
        (true, Some(DeviceStatus::Disabled)) => "device.disable",
        (true, Some(DeviceStatus::Active)) => "device.enable",
        _ => "device.update",
    }
}

/// Deletes the device and its strategy assignment (audited `device.delete`). The device is
/// not blocked: it registers as a new device again, a UDP device within seconds and a
/// device in TCP mode only when its connection to the signal server is next re-established.
#[utoipa::path(
    delete,
    path = "/devices/{id}",
    tag = "devices",
    operation_id = "delete_device",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Device id")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "No such device", body = Problem),
    )
)]
pub async fn delete_device(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let guard = device_guard(&state, &ctx, "device.delete", id);
    match state.devices.delete_guarded(id, guard).await? {
        true => Ok((Audited, StatusCode::NO_CONTENT)),
        false => Err(AdminApiError::not_found("device not found")),
    }
}

/// Asks the device to close one of its connections. The request waits for the device's next
/// heartbeat (every 3 s while it has connections) and expires after `ttl_seconds`; a
/// connection the device does not report stays queued until then (audited
/// `device.disconnect`).
#[utoipa::path(
    post,
    path = "/devices/{id}/disconnect",
    tag = "devices",
    operation_id = "disconnect_device",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Device id")),
    request_body = DisconnectBody,
    responses(
        (status = 202, description = "Queued", body = DisconnectQueued),
        (status = 404, description = "No such device", body = Problem),
        (status = 503, description = "Too many devices have pending requests (`unavailable`)", body = Problem),
    )
)]
pub async fn disconnect_device(
    State(state): State<Arc<AppState>>,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<i64>,
    ValidJson(body): ValidJson<DisconnectBody>,
) -> Result<(StatusCode, Audited, Json<DisconnectQueued>), AdminApiError> {
    let guard = disconnect_guard(&state, &ctx, id, body.conn_id);
    state
        .devices
        .disconnect_guarded(id, body.conn_id, guard)
        .await?;
    let queued = DisconnectQueued {
        ttl_seconds: DISCONNECT_TTL.as_secs(),
    };
    Ok((StatusCode::ACCEPTED, Audited, Json(queued)))
}
