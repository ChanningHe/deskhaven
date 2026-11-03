//! Connection records (03 §2.3) merged into `connection_sessions`.
//!
//! A connection is reported in three records, each of which may arrive first:
//!
//! | record | session id | fills |
//! |---|---|---|
//! | opening (`action: "new"`) | 0 (sent before the login request) | `ip`, `started_at` |
//! | authorization (`peer`, `type`, ...) | the controller's | peer, auth, `authorized_at` |
//! | close (`action: "close"`) | the controller's (0 if closed before login) | `ended_at` |
//!
//! The first record creates the row; the others find it by the unique key (device,
//! session id, conn id) or, across the 0 / real session id divide, **pair** with it: same
//! device and conn id, created within [`PAIRING_WINDOW`], its slot for this record still
//! empty. A record with the real session id adopts an unbound, unclosed row (and binds it to
//! the session); a late opening joins a bound row that has no opening yet. Conn ids are a
//! per-process counter of the client, so the window keeps a reused id (after a client
//! restart) from joining an old row.
//!
//! Each slot is filled once: the first record of a kind wins (06 §7, audit records are
//! append-only for the API). Times are the server's receipt times; the client sends none.

use std::net::IpAddr;
use std::time::Duration;

use deskhaven_db::entity::connection_session::{self, ActiveModel, Column, Entity, Model};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect,
};

use super::SessionId;
use super::session_id::UNBOUND;
use crate::text;

/// How long after a connection's first record its other records still pair with it across
/// the session id divide: a login may take 180 s (`LOGIN_GRACE`,
/// `src/server/connection.rs:89`) and each record up to 120 s of retries.
pub const PAIRING_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Longest stored peer id (hbbs `MAX_PEER_ID_LEN`) and peer name, in characters.
const PEER_ID_MAX_CHARS: usize = 253;
const PEER_NAME_MAX_CHARS: usize = 256;

/// One connection record of the controlled device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnReport {
    /// The device's connection id (`Connection.inner.id`).
    pub conn_id: i64,
    pub session_id: SessionId,
    pub event: ConnEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnEvent {
    /// The connection opened, from `ip`.
    New {
        ip: Option<IpAddr>,
    },
    /// The controller logged in.
    Authorized(Authorization),
    Close,
}

/// Who logged in and how (`src/server/connection.rs:1944-1952`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    /// The controller's RustDesk id and name (`LoginRequest.my_id` / `my_name`).
    pub peer_id: String,
    pub peer_name: String,
    /// 0 remote control, 1 file transfer, 2 port forward, 3 camera, 4 terminal; newer clients
    /// may send other values, stored as they come.
    pub conn_type: i32,
    /// 1 click, 2 temporary password, 3 permanent password, 4 switch sides.
    pub primary_auth: Option<i32>,
    /// 1 TOTP, 2 trusted device.
    pub two_factor: Option<i32>,
}

/// Merges `report` of `device` into its row in `tx`, creating it if it is the first record.
pub(super) async fn merge(
    tx: &DatabaseTransaction,
    device: &str,
    report: &ConnReport,
    now: TimestampMs,
) -> Result<(), DbErr> {
    let found = match find_exact(tx, device, report).await? {
        Some(row) => Some(row),
        None => find_partner(tx, device, report, now).await?,
    };
    match found {
        Some(row) => update(tx, row, report, now).await,
        None => insert(tx, device, report, now).await,
    }
}

async fn find_exact(
    tx: &DatabaseTransaction,
    device: &str,
    report: &ConnReport,
) -> Result<Option<Model>, DbErr> {
    Entity::find()
        .filter(Column::DeviceRustdeskId.eq(device))
        .filter(Column::SessionId.eq(report.session_id.as_str()))
        .filter(Column::ConnId.eq(report.conn_id))
        .lock_exclusive()
        .one(tx)
        .await
}

/// The row of the same connection on the other side of the session id divide (see the
/// module documentation), newest first.
async fn find_partner(
    tx: &DatabaseTransaction,
    device: &str,
    report: &ConnReport,
    now: TimestampMs,
) -> Result<Option<Model>, DbErr> {
    let unbound = report.session_id.is_unbound();
    // An unbound close (or authorization) belongs to a connection that never logged in.
    if unbound && !matches!(report.event, ConnEvent::New { .. }) {
        return Ok(None);
    }
    let window = i64::try_from(PAIRING_WINDOW.as_millis()).unwrap_or(i64::MAX);
    let since = time::from_millis(now.timestamp_millis().saturating_sub(window));
    let mut query = Entity::find()
        .filter(Column::DeviceRustdeskId.eq(device))
        .filter(Column::ConnId.eq(report.conn_id))
        .filter(slot(&report.event).is_null());
    if let Some(since) = since {
        query = query.filter(Column::CreatedAt.gte(since));
    }
    query = match unbound {
        true => query.filter(Column::SessionId.ne(UNBOUND)),
        false => query
            .filter(Column::SessionId.eq(UNBOUND))
            .filter(Column::EndedAt.is_null()),
    };
    query
        .order_by_desc(Column::CreatedAt)
        .order_by_desc(Column::Id)
        .lock_exclusive()
        .one(tx)
        .await
}

/// The column that records `event` (empty until it arrives).
fn slot(event: &ConnEvent) -> Column {
    match event {
        ConnEvent::New { .. } => Column::StartedAt,
        ConnEvent::Authorized(_) => Column::AuthorizedAt,
        ConnEvent::Close => Column::EndedAt,
    }
}

fn is_filled(row: &Model, event: &ConnEvent) -> bool {
    match event {
        ConnEvent::New { .. } => row.started_at.is_some(),
        ConnEvent::Authorized(_) => row.authorized_at.is_some(),
        ConnEvent::Close => row.ended_at.is_some(),
    }
}

async fn update(
    tx: &DatabaseTransaction,
    row: Model,
    report: &ConnReport,
    now: TimestampMs,
) -> Result<(), DbErr> {
    let filled = is_filled(&row, &report.event);
    let binds = row.session_id != report.session_id.as_str() && !report.session_id.is_unbound();
    if filled && !binds {
        tracing::debug!(conn_id = report.conn_id, "audit record already merged");
        return Ok(());
    }
    let mut active = row.into_active_model();
    if binds {
        active.session_id = Set(report.session_id.as_str().to_owned());
    }
    if !filled {
        fill(&mut active, &report.event, now);
    }
    active.update(tx).await.map(drop)
}

async fn insert(
    tx: &DatabaseTransaction,
    device: &str,
    report: &ConnReport,
    now: TimestampMs,
) -> Result<(), DbErr> {
    let mut row = ActiveModel {
        guid: Set(uuid::Uuid::now_v7().to_string()),
        device_rustdesk_id: Set(device.to_owned()),
        conn_id: Set(report.conn_id),
        session_id: Set(report.session_id.as_str().to_owned()),
        created_at: Set(now),
        ..Default::default()
    };
    fill(&mut row, &report.event, now);
    connection_session::Entity::insert(row)
        .exec_without_returning(tx)
        .await
        .map(drop)
}

/// Sets the columns `event` reports.
fn fill(row: &mut ActiveModel, event: &ConnEvent, now: TimestampMs) {
    match event {
        ConnEvent::New { ip } => {
            row.ip = Set(ip.map(|ip| ip.to_string()));
            row.started_at = Set(Some(now));
        }
        ConnEvent::Authorized(auth) => {
            let peer_id = text::clean(&auth.peer_id, PEER_ID_MAX_CHARS, false);
            let peer_name = text::clean(&auth.peer_name, PEER_NAME_MAX_CHARS, false);
            row.peer_rustdesk_id = Set(Some(peer_id));
            row.peer_name = Set(Some(peer_name));
            row.conn_type = Set(Some(auth.conn_type));
            row.primary_auth = Set(auth.primary_auth);
            row.two_factor = Set(auth.two_factor);
            row.authorized_at = Set(Some(now));
        }
        ConnEvent::Close => row.ended_at = Set(Some(now)),
    }
}
