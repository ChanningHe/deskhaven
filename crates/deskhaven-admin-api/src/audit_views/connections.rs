//! `GET /audit/connections`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::audit_query::{AuditQuery, ConnectionEntry, ConnectionQuery};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::filter::{MAX_LABEL_CHARS, MAX_Q_CHARS, WindowQuery, trimmed};
use super::view::AuditUserView;
use crate::{AdminApiError, PageQuery, Paginated, ValidQuery};

/// Filters of the connection list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ConnectionFilter {
    /// The controlled device's RustDesk id.
    #[garde(length(chars, max = MAX_LABEL_CHARS))]
    #[param(max_length = 256)]
    pub device: Option<String>,
    /// The controlling user account.
    #[garde(skip)]
    pub user: Option<i64>,
    /// Connection type code: 0 remote control, 1 file transfer, 2 port forward, 3 camera,
    /// 4 terminal (newer clients may send others).
    #[garde(range(min = 0, max = 1000))]
    #[param(minimum = 0, maximum = 1000)]
    pub conn_type: Option<i32>,
    /// Substring of the peer id, peer name or address (case-insensitive, literal).
    #[garde(length(chars, max = MAX_Q_CHARS))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

/// One connection, as the controlled device reported it (everything but `id`, `guid`,
/// `controller` and `created_at` is device self-reported).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ConnectionAuditView {
    pub id: i64,
    pub guid: String,
    /// The controlled device's RustDesk id.
    pub device_id: String,
    pub conn_id: i64,
    /// `null` for a retired row: its session id was given up to a newer connection.
    pub session_id: Option<String>,
    pub peer_id: Option<String>,
    pub peer_name: Option<String>,
    /// The controlling account; `null` when unknown or deleted.
    pub controller: Option<AuditUserView>,
    /// 0 remote control, 1 file transfer, 2 port forward, 3 camera, 4 terminal.
    pub conn_type: Option<i32>,
    /// 1 click to accept, 2 temporary password, 3 permanent password, 4 switch sides.
    pub primary_auth: Option<i32>,
    /// 1 TOTP, 2 trusted device.
    pub two_factor: Option<i32>,
    pub ip: Option<String>,
    pub started_at: Option<i64>,
    pub authorized_at: Option<i64>,
    pub ended_at: Option<i64>,
    /// Written on the controlled side.
    pub note: Option<String>,
    /// Written by the controller.
    pub controller_note: Option<String>,
    /// When the first message of the connection arrived.
    pub created_at: i64,
}

impl From<ConnectionEntry> for ConnectionAuditView {
    fn from(entry: ConnectionEntry) -> Self {
        let session_id = entry.session_id().map(str::to_owned);
        let row = entry.row;
        Self {
            id: row.id,
            guid: row.guid,
            device_id: row.device_rustdesk_id,
            conn_id: row.conn_id,
            session_id,
            peer_id: row.peer_rustdesk_id,
            peer_name: row.peer_name,
            controller: entry.controller.map(Into::into),
            conn_type: row.conn_type,
            primary_auth: row.primary_auth,
            two_factor: row.two_factor,
            ip: row.ip,
            started_at: row.started_at.map(|t| t.timestamp_millis()),
            authorized_at: row.authorized_at.map(|t| t.timestamp_millis()),
            ended_at: row.ended_at.map(|t| t.timestamp_millis()),
            note: row.note,
            controller_note: row.controller_note,
            created_at: row.created_at.timestamp_millis(),
        }
    }
}

/// Connections, newest first by default.
#[utoipa::path(
    get,
    path = "/audit/connections",
    tag = "audit",
    operation_id = "list_connection_audit",
    security(("console_session" = ["admin"])),
    params(PageQuery, WindowQuery, ConnectionFilter),
    responses((status = 200, description = "One page of connections", body = Paginated<ConnectionAuditView>))
)]
pub async fn list_connection_audit(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(window): ValidQuery<WindowQuery>,
    ValidQuery(filter): ValidQuery<ConnectionFilter>,
) -> Result<Json<Paginated<ConnectionAuditView>>, AdminApiError> {
    let query = ConnectionQuery {
        listing: window.listing(page)?,
        device: trimmed(filter.device),
        controller_user_id: filter.user,
        conn_type: filter.conn_type,
        q: filter.q,
    };
    let found = AuditQuery::new(state.db.clone())
        .connections(&query)
        .await?;
    Ok(Json(Paginated::from_page(found, Into::into)))
}
