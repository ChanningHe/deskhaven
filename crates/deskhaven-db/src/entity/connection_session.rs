//! `connection_sessions`: one remote connection as reported by the controlled device in up to
//! three audit messages (new, authorized, close) plus the controller's note.
//!
//! The messages may arrive in any order and the first one creates the row, so everything but
//! the identity of the connection is optional. No foreign key to `devices`: the audit trail
//! outlives the device record.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "connection_sessions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// UUIDv7 handed to clients for `conn/active` and notes.
    pub guid: String,
    /// The controlled device.
    pub device_rustdesk_id: String,
    pub conn_id: i64,
    /// Client-supplied; text because it is opaque to the server.
    pub session_id: String,
    /// The controlling device.
    pub peer_rustdesk_id: Option<String>,
    pub peer_name: Option<String>,
    /// Resolved from the audit reference; cleared when the user is deleted.
    pub controller_user_id: Option<i64>,
    /// 0 remote control, 1 file transfer, 2 port forward, 3 camera, 4 terminal; unconstrained,
    /// newer clients may send other values.
    pub conn_type: Option<i32>,
    pub primary_auth: Option<i32>,
    pub two_factor: Option<i32>,
    pub ip: Option<String>,
    pub started_at: Option<TimestampMs>,
    pub authorized_at: Option<TimestampMs>,
    pub ended_at: Option<TimestampMs>,
    /// Note written on the controlled side.
    pub note: Option<String>,
    pub controller_note: Option<String>,
    /// When the first message of this connection arrived.
    pub created_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::ControllerUserId",
        to = "super::user::Column::Id",
        on_delete = "SetNull"
    )]
    User,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
