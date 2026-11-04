//! Connection sessions (`connection_sessions`, 03 §2.3-§2.4).

use deskhaven_db::entity::connection_session::{Column, Entity, Model};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::listing::{Listing, text};
use super::names::{AuditUser, user_of, usernames};
use super::{AuditQuery, AuditQueryError};
use crate::audit::RETIRED_PREFIX;
use crate::pagination::Page;

/// Filters of [`AuditQuery::connections`]. `q` matches the peer id, peer name and address.
#[derive(Clone, Debug, Default)]
pub struct ConnectionQuery {
    pub listing: Listing,
    /// The controlled device's RustDesk id.
    pub device: Option<String>,
    pub controller_user_id: Option<i64>,
    pub conn_type: Option<i32>,
    pub q: Option<String>,
}

/// A connection with its controller, when that account still exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionEntry {
    pub row: Model,
    pub controller: Option<AuditUser>,
}

impl ConnectionEntry {
    /// The session id the client reported. `None` for a retired row (its id was given up
    /// to a newer connection with the same session id, see `audit::RETIRED_PREFIX`): the
    /// stored `0#<row id>` is bookkeeping, not data.
    pub fn session_id(&self) -> Option<&str> {
        let id = self.row.session_id.as_str();
        (!id.starts_with(RETIRED_PREFIX)).then_some(id)
    }
}

impl AuditQuery {
    pub async fn connections(
        &self,
        query: &ConnectionQuery,
    ) -> Result<Page<ConnectionEntry>, AuditQueryError> {
        let mut select = Entity::find();
        if let Some(device) = &query.device {
            select = select.filter(Column::DeviceRustdeskId.eq(device.as_str()));
        }
        if let Some(user) = query.controller_user_id {
            select = select.filter(Column::ControllerUserId.eq(user));
        }
        if let Some(kind) = query.conn_type {
            select = select.filter(Column::ConnType.eq(kind));
        }
        if let Some(q) = text(query.q.as_deref()) {
            let columns = [Column::PeerRustdeskId, Column::PeerName, Column::Ip];
            select = select.filter(self.contains(&columns, q));
        }
        let mut page = query
            .listing
            .run(&self.db, select, (Column::CreatedAt, Column::Id))
            .await?;
        let ids = page.items.iter().filter_map(|row| row.controller_user_id);
        let names = usernames(&self.db, ids).await?;
        let rows = std::mem::take(&mut page.items);
        let entries = rows
            .into_iter()
            .map(|row| ConnectionEntry {
                controller: user_of(&names, row.controller_user_id),
                row,
            })
            .collect();
        Ok(page.with_items(entries))
    }
}
