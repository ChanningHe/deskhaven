//! Alarms (`alarms`, 03 §2.6).

use deskhaven_db::entity::alarm::{Column, Entity, Model};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::Value;

use super::listing::Listing;
use super::{AuditQuery, AuditQueryError};
use crate::pagination::Page;

/// Filters of [`AuditQuery::alarms`].
#[derive(Clone, Debug, Default)]
pub struct AlarmQuery {
    pub listing: Listing,
    /// The reporting device's RustDesk id.
    pub device: Option<String>,
    /// The alarm type code as the client sent it.
    pub typ: Option<i32>,
}

/// The `key = value` pairs of a stored alarm `info` (a flat JSON object, in key order).
/// Strings are taken as they are, other scalars and nested values as their JSON text; a
/// column that is not an object reads as empty: the device wrote it.
pub fn alarm_info(info: &str) -> Vec<(String, String)> {
    let Ok(Value::Object(members)) = serde_json::from_str::<Value>(info) else {
        return Vec::new();
    };
    members
        .into_iter()
        .map(|(key, value)| match value {
            Value::String(text) => (key, text),
            other => (key, other.to_string()),
        })
        .collect()
}

impl AuditQuery {
    pub async fn alarms(&self, query: &AlarmQuery) -> Result<Page<Model>, AuditQueryError> {
        let mut select = Entity::find();
        if let Some(device) = &query.device {
            select = select.filter(Column::DeviceRustdeskId.eq(device.as_str()));
        }
        if let Some(typ) = query.typ {
            select = select.filter(Column::Typ.eq(typ));
        }
        query
            .listing
            .run(&self.db, select, (Column::CreatedAt, Column::Id))
            .await
    }
}
