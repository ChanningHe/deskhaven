//! File transfers (`file_transfers`, 03 §2.5).

use deskhaven_db::entity::file_transfer::{Column, Direction, Entity, Model};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;

use super::listing::{Listing, text};
use super::{AuditQuery, AuditQueryError};
use crate::pagination::Page;

/// Filters of [`AuditQuery::file_transfers`]. `q` matches the path, peer id and address.
#[derive(Clone, Debug, Default)]
pub struct FileQuery {
    pub listing: Listing,
    /// The reporting (controlled) device's RustDesk id.
    pub device: Option<String>,
    pub direction: Option<Direction>,
    pub q: Option<String>,
}

/// One file of a transfer, as the device named it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferredFile {
    pub name: String,
    pub size: i64,
}

/// The files of a stored `files` column (`[[name, size], ...]`). Entries that do not have
/// that shape are dropped, and a column that is not an array reads as empty: the device
/// wrote it, and an odd one must not break the list.
pub fn transferred_files(files: &str) -> Vec<TransferredFile> {
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(files) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let pair = entry.as_array()?;
            Some(TransferredFile {
                name: pair.first()?.as_str()?.to_owned(),
                size: pair.get(1)?.as_i64()?,
            })
        })
        .collect()
}

impl AuditQuery {
    pub async fn file_transfers(&self, query: &FileQuery) -> Result<Page<Model>, AuditQueryError> {
        let mut select = Entity::find();
        if let Some(device) = &query.device {
            select = select.filter(Column::DeviceRustdeskId.eq(device.as_str()));
        }
        if let Some(direction) = query.direction {
            select = select.filter(Column::Direction.eq(direction));
        }
        if let Some(q) = text(query.q.as_deref()) {
            let columns = [Column::Path, Column::PeerRustdeskId, Column::Ip];
            select = select.filter(self.contains(&columns, q));
        }
        query
            .listing
            .run(&self.db, select, (Column::CreatedAt, Column::Id))
            .await
    }
}
