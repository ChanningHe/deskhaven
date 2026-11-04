//! `GET /audit/alarms`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::audit_query::{AlarmQuery, AlarmRow, AuditQuery, alarm_info};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::filter::{MAX_LABEL_CHARS, WindowQuery, plain_text, trimmed};
use crate::{AdminApiError, PageQuery, Paginated, ValidQuery};

/// Filters of the alarm list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AlarmFilter {
    /// The reporting device's RustDesk id.
    #[garde(length(chars, max = MAX_LABEL_CHARS), custom(plain_text))]
    #[param(max_length = 256)]
    pub device: Option<String>,
    /// Alarm type code: 0 IP allow-list block, 1 over 30 attempts, 2 six attempts in a
    /// minute, 6 IPv6 prefix attempts, 7 and 8 terminal OS login, 9 session scope
    /// violation, 10 ID allow-list (newer clients may send others).
    #[garde(range(min = 0, max = 1000))]
    #[param(minimum = 0, maximum = 1000)]
    pub typ: Option<i32>,
}

/// One entry of an alarm's `info`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AlarmInfoEntry {
    pub key: String,
    /// Strings as sent; other values as JSON text.
    pub value: String,
}

/// An alarm; all of it is device self-reported except `id` and `created_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AlarmAuditView {
    pub id: i64,
    pub device_id: String,
    pub typ: i32,
    /// The device's key-value details, in key order.
    pub info: Vec<AlarmInfoEntry>,
    pub conn_id: i64,
    pub created_at: i64,
}

impl From<AlarmRow> for AlarmAuditView {
    fn from(row: AlarmRow) -> Self {
        Self {
            id: row.id,
            device_id: row.device_rustdesk_id,
            typ: row.typ,
            info: alarm_info(&row.info)
                .into_iter()
                .map(|(key, value)| AlarmInfoEntry { key, value })
                .collect(),
            conn_id: row.conn_id,
            created_at: row.created_at.timestamp_millis(),
        }
    }
}

/// Alarms, newest first by default.
#[utoipa::path(
    get,
    path = "/audit/alarms",
    tag = "audit",
    operation_id = "list_alarm_audit",
    security(("console_session" = ["admin"])),
    params(PageQuery, WindowQuery, AlarmFilter),
    responses((status = 200, description = "One page of alarms", body = Paginated<AlarmAuditView>))
)]
pub async fn list_alarm_audit(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(window): ValidQuery<WindowQuery>,
    ValidQuery(filter): ValidQuery<AlarmFilter>,
) -> Result<Json<Paginated<AlarmAuditView>>, AdminApiError> {
    let query = AlarmQuery {
        listing: window.listing(page)?,
        device: trimmed(filter.device),
        typ: filter.typ,
    };
    let found = AuditQuery::new(state.db.clone()).alarms(&query).await?;
    Ok(Json(Paginated::from_page(found, Into::into)))
}
