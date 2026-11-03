//! `GET /devices/{id}`: one device with its live connections, effective strategy and
//! recent connections.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::state::AppState;
use serde::Serialize;
use utoipa::ToSchema;

use super::strategy::{EffectiveStrategyView, effective_strategy};
use super::view::{ConnectionView, DeviceView};
use crate::{AdminApiError, ValidPath};

type Problem = crate::ErrorEnvelope<'static>;

/// Connections listed in the detail.
const RECENT_CONNECTIONS: u64 = 10;

/// A device with everything the console's drawer shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DeviceDetail {
    pub device: DeviceView,
    /// Source address and time of the last signal registration.
    pub register_ip: String,
    pub register_at: i64,
    /// Connection ids the device reported in its latest heartbeat (the ones that can be
    /// disconnected).
    pub connections: Vec<i32>,
    pub strategy: EffectiveStrategyView,
    /// Newest first; filled by the audit ingest, so it can be empty.
    pub recent_connections: Vec<ConnectionView>,
}

/// One device with its live connections, effective strategy and recent connections.
#[utoipa::path(
    get,
    path = "/devices/{id}",
    tag = "devices",
    operation_id = "get_device",
    security(("console_session" = ["admin"])),
    params(("id" = i64, Path, description = "Device id")),
    responses(
        (status = 200, description = "The device", body = DeviceDetail),
        (status = 404, description = "No such device", body = Problem),
    )
)]
pub async fn get_device(
    State(state): State<Arc<AppState>>,
    ValidPath(id): ValidPath<i64>,
) -> Result<Json<DeviceDetail>, AdminApiError> {
    let summary = state.devices.summary(id).await?;
    let strategy = effective_strategy(&state, &summary.record).await?;
    let recent = state
        .devices
        .recent_connections(&summary.record.rustdesk_id, RECENT_CONNECTIONS)
        .await?;
    let connections = summary
        .presence
        .as_ref()
        .map(|p| p.conns.clone())
        .unwrap_or_default();
    let (register_ip, register_at) = (
        summary.record.register_ip.clone(),
        summary.record.register_at.timestamp_millis(),
    );
    Ok(Json(DeviceDetail {
        device: summary.into(),
        register_ip,
        register_at,
        connections,
        strategy,
        recent_connections: recent.into_iter().map(Into::into).collect(),
    }))
}
