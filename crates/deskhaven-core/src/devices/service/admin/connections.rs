//! A device's recent connections, read from `connection_sessions` (written by the audit
//! ingest). The table has no foreign key to `devices`: the history outlives the device.

use deskhaven_db::entity::connection_session;
use deskhaven_db::time::TimestampMs;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::devices::DeviceError;
use crate::devices::service::DeviceService;

/// One connection to a device, as the device reported it. Everything but the identity is
/// optional: the messages of a connection arrive in any order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionRecord {
    pub guid: String,
    pub conn_id: i64,
    /// The controlling device and the name it gave.
    pub peer_rustdesk_id: Option<String>,
    pub peer_name: Option<String>,
    pub controller_user_id: Option<i64>,
    /// 0 remote control, 1 file transfer, 2 port forward, 3 camera, 4 terminal.
    pub conn_type: Option<i32>,
    pub ip: Option<String>,
    pub started_at: Option<TimestampMs>,
    pub authorized_at: Option<TimestampMs>,
    pub ended_at: Option<TimestampMs>,
    pub created_at: TimestampMs,
}

impl From<connection_session::Model> for ConnectionRecord {
    fn from(m: connection_session::Model) -> Self {
        Self {
            guid: m.guid,
            conn_id: m.conn_id,
            peer_rustdesk_id: m.peer_rustdesk_id,
            peer_name: m.peer_name,
            controller_user_id: m.controller_user_id,
            conn_type: m.conn_type,
            ip: m.ip,
            started_at: m.started_at,
            authorized_at: m.authorized_at,
            ended_at: m.ended_at,
            created_at: m.created_at,
        }
    }
}

impl DeviceService {
    /// The latest `limit` connections to `rustdesk_id`, newest first.
    pub async fn recent_connections(
        &self,
        rustdesk_id: &str,
        limit: u64,
    ) -> Result<Vec<ConnectionRecord>, DeviceError> {
        let rows = connection_session::Entity::find()
            .filter(connection_session::Column::DeviceRustdeskId.eq(rustdesk_id))
            .order_by_desc(connection_session::Column::CreatedAt)
            .order_by_desc(connection_session::Column::Id)
            .limit(limit)
            .all(&self.inner.db)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }
}
