//! A connection is reported in three records (opening, authorization, close), each of which
//! may arrive first; the opening carries session id 0. A record finds its row by the unique key
//! or pairs with one across the 0 / real session id divide within [`PAIRING_WINDOW`], which
//! keeps a conn id the client reused after a restart from joining an old row. An unbound row of
//! an earlier connection is retired (re-keyed, contents kept). Each slot is filled once: the
//! first record of a kind wins.

use std::net::IpAddr;
use std::time::Duration;

use deskhaven_db::entity::connection_session::{self, ActiveModel, Column, Entity, Model};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbBackend, DbErr,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Statement,
};

use super::SessionId;
use super::session_id::UNBOUND;
use crate::text;

/// A login may take 180 s and each record up to 120 s of retries.
pub const PAIRING_WINDOW: Duration = Duration::from_secs(10 * 60);
/// Longest stored peer id (hbbs `MAX_PEER_ID_LEN`) and peer name, in characters.
const PEER_ID_MAX_CHARS: usize = 253;
const PEER_NAME_MAX_CHARS: usize = 256;
/// Not a decimal, so it never meets a client's session id.
pub const RETIRED_PREFIX: &str = "0#";
const PG_CONN_LOCK_NS: i32 = 0x6468_6163;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnReport {
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
    Authorized(Authorization),
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
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

pub(super) async fn merge(
    tx: &DatabaseTransaction,
    device: &str,
    report: &ConnReport,
    now: TimestampMs,
) -> Result<(), DbErr> {
    lock_connection(tx, device, report.conn_id).await?;
    if let Some(row) = find_exact(tx, device, report).await? {
        if same_connection(&row, report, now) {
            return update(tx, row, report, now).await;
        }
        retire(tx, row).await?;
    }
    match find_partner(tx, device, report, now).await? {
        Some(row) => update(tx, row, report, now).await,
        None => insert(tx, device, report, now).await,
    }
}

/// Under READ COMMITTED an opening and an authorization arriving at once would both find no
/// row and insert two rows that never merge. SQLite's `BEGIN IMMEDIATE` already serializes.
async fn lock_connection(
    tx: &DatabaseTransaction,
    device: &str,
    conn_id: i64,
) -> Result<(), DbErr> {
    if tx.get_database_backend() != DbBackend::Postgres {
        return Ok(());
    }
    let sql = "SELECT pg_advisory_xact_lock($1, hashtext($2))";
    let values = [
        PG_CONN_LOCK_NS.into(),
        // Unambiguous: the number ends at the first ':' (PostgreSQL text cannot hold NUL).
        format!("{conn_id}:{device}").into(),
    ];
    let statement = Statement::from_sql_and_values(DbBackend::Postgres, sql, values);
    tx.execute_raw(statement).await.map(drop)
}

fn same_connection(row: &Model, report: &ConnReport, now: TimestampMs) -> bool {
    !report.session_id.is_unbound()
        || (within_window(row.created_at, now) && !is_filled(row, &report.event))
}

async fn retire(tx: &DatabaseTransaction, row: Model) -> Result<(), DbErr> {
    tracing::debug!(
        conn_id = row.conn_id,
        "retiring an earlier connection's unbound row"
    );
    let retired = format!("{RETIRED_PREFIX}{}", row.id);
    let mut active = row.into_active_model();
    active.session_id = Set(retired);
    active.update(tx).await.map(drop)
}

fn within_window(created_at: TimestampMs, now: TimestampMs) -> bool {
    let window = i64::try_from(PAIRING_WINDOW.as_millis()).unwrap_or(i64::MAX);
    now.timestamp_millis()
        .saturating_sub(created_at.timestamp_millis())
        <= window
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
        true => query
            .filter(Column::SessionId.ne(UNBOUND))
            .filter(Column::SessionId.not_like(format!("{RETIRED_PREFIX}%"))),
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
