//! `GET /overview` (M3-14): the aggregate numbers of the console's landing page, in one
//! read-only admin call. Online devices and live connections come from the in-memory
//! presence table; the rest are index range scans and two aggregates, so the call stays
//! cheap enough for the page to poll it.
//!
//! The client versions and the alarm texts are device self-reported (06 §2); the console
//! renders them as text.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::audit_query::AuditQuery;
use deskhaven_core::devices::{FleetStats, VersionCount};
use deskhaven_core::state::AppState;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::routes;

use crate::audit_views::AlarmAuditView;
use crate::{AdminApiError, AdminRoutes, RouteError};

/// Window of the connection and failed login counts.
const WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// Alarms listed.
const RECENT_ALARMS: u64 = 5;
/// Client versions listed before "other".
const TOP_VERSIONS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct DeviceTotals {
    /// Devices online now (signal registration or heartbeat within their liveness window).
    pub online: u64,
    pub total: u64,
}

/// Devices that report one client version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct VersionShare {
    /// As the device reported it: untrusted text.
    pub version: String,
    pub count: u64,
}

/// How the client versions the devices report are spread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ClientVersions {
    /// The most common versions, most devices first.
    pub versions: Vec<VersionShare>,
    /// Devices on a version that is not listed.
    pub other: u64,
    /// Devices that never reported a version.
    pub unknown: u64,
}

/// The overview page's numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct OverviewView {
    /// When the numbers were taken (unix ms).
    pub generated_at: i64,
    pub devices: DeviceTotals,
    /// Connections the online devices report right now.
    pub active_connections: u64,
    /// Connections that started in the last 24 hours.
    pub connections_24h: u64,
    /// Failed login attempts (console and client) in the last 24 hours.
    pub failed_logins_24h: u64,
    /// The latest alarms, newest first.
    pub recent_alarms: Vec<AlarmAuditView>,
    /// Self-reported by the devices.
    pub client_versions: ClientVersions,
}

impl From<VersionCount> for VersionShare {
    fn from(count: VersionCount) -> Self {
        Self {
            version: count.version,
            count: count.count,
        }
    }
}

fn client_versions(fleet: &FleetStats) -> ClientVersions {
    ClientVersions {
        versions: fleet.top_versions.iter().cloned().map(Into::into).collect(),
        other: fleet.other_versions,
        unknown: fleet.unknown_versions,
    }
}

/// The overview: device and connection counts, 24 hour activity, latest alarms and the
/// client version distribution.
#[utoipa::path(
    get,
    path = "/overview",
    tag = "overview",
    operation_id = "get_overview",
    security(("console_session" = ["admin"])),
    responses((status = 200, description = "The overview numbers", body = OverviewView))
)]
pub async fn get_overview(
    State(state): State<Arc<AppState>>,
) -> Result<Json<OverviewView>, AdminApiError> {
    let now = state.clock.now_ms();
    let audit = AuditQuery::new(state.db.clone());
    let fleet = state.devices.fleet(TOP_VERSIONS).await?;
    let activity = audit.activity_since(now - WINDOW_MS).await?;
    let alarms = audit.latest_alarms(RECENT_ALARMS).await?;
    Ok(Json(OverviewView {
        generated_at: now,
        devices: DeviceTotals {
            online: fleet.online,
            total: fleet.total,
        },
        active_connections: fleet.connections,
        connections_24h: activity.connections,
        failed_logins_24h: activity.failed_logins,
        recent_alarms: alarms.into_iter().map(Into::into).collect(),
        client_versions: client_versions(&fleet),
    }))
}

/// The `/overview` route.
pub(crate) fn routes(r: AdminRoutes) -> Result<AdminRoutes, RouteError> {
    r.routes(routes!(get_overview))
}
