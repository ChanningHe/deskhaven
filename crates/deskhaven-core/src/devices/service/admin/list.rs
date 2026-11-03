//! Paged device list and single device lookup, with the owner and group names and the
//! presence of each device.
//!
//! **Online filter**: online status lives in memory ([`Presence`](crate::devices::Presence)),
//! not in the database. With `online` set, the ids matching the other filters are read
//! (two columns, in order), filtered through the presence table and paged in memory; without
//! it the database pages. Either way the page is the same shape.

use std::collections::HashMap;

use deskhaven_db::entity::device::{self, DeviceStatus};
use deskhaven_db::entity::{device_group, user};
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Select,
};

use super::DeviceRecord;
use crate::devices::DeviceError;
use crate::devices::presence::DevicePresence;
use crate::devices::service::{DeviceService, Inner};
use crate::pagination::{Page, PageRequest};
use crate::text::contains_pattern;

/// Filters and order of [`DeviceService::page`]. `q` matches a substring of the RustDesk id,
/// hostname, device username or note, `os` a substring of the reported OS; both
/// case-insensitively (ASCII on SQLite) and literally.
#[derive(Clone, Debug, Default)]
pub struct DeviceQuery {
    pub q: Option<String>,
    pub os: Option<String>,
    pub owner_user_id: Option<i64>,
    pub device_group_id: Option<i64>,
    pub status: Option<DeviceStatus>,
    /// `Some(true)`: online now; `Some(false)`: not.
    pub online: Option<bool>,
    pub sort: Option<DeviceSort>,
    pub page: PageRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceSortField {
    RustdeskId,
    Hostname,
    Os,
    Version,
    Status,
    LastSeenAt,
    CreatedAt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceSort {
    pub field: DeviceSortField,
    pub descending: bool,
}

impl DeviceSortField {
    fn column(self) -> device::Column {
        match self {
            Self::RustdeskId => device::Column::RustdeskId,
            Self::Hostname => device::Column::Hostname,
            Self::Os => device::Column::Os,
            Self::Version => device::Column::Version,
            Self::Status => device::Column::Status,
            Self::LastSeenAt => device::Column::LastSeenAt,
            Self::CreatedAt => device::Column::CreatedAt,
        }
    }
}

/// A device with the names of its owner and group and what the presence table knows of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    pub record: DeviceRecord,
    pub owner_username: Option<String>,
    pub group_name: Option<String>,
    /// `None`: not tracked (never seen since the last restart, or evicted).
    pub presence: Option<DevicePresence>,
}

impl DeviceService {
    /// One page of devices (ordered by `sort`, default RustDesk id ascending, then id in the
    /// same direction).
    pub async fn page(&self, query: &DeviceQuery) -> Result<Page<DeviceSummary>, DeviceError> {
        let index = query.page.index().map_err(|p| DeviceError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let inner = &self.inner;
        let sort = query.sort.unwrap_or(DEFAULT_SORT);
        let select = filtered(query);
        let (total, rows) = match query.online {
            None => inner.page_in_db(select, sort, query.page, index).await?,
            Some(online) => {
                let wanted = inner.online_ids(select, sort, online).await?;
                inner.page_of_ids(&wanted, query.page, index).await?
            }
        };
        Ok(Page {
            items: inner.summaries(rows).await?,
            total,
            page: query.page.page,
            page_size: query.page.page_size,
        })
    }

    /// The device with `devices.id` `id`.
    pub async fn summary(&self, id: i64) -> Result<DeviceSummary, DeviceError> {
        let found = device::Entity::find_by_id(id).one(&self.inner.db).await?;
        let row = found.ok_or(DeviceError::NotFound)?;
        let mut summaries = self.inner.summaries(vec![row]).await?;
        summaries.pop().ok_or(DeviceError::NotFound)
    }
}

const DEFAULT_SORT: DeviceSort = DeviceSort {
    field: DeviceSortField::RustdeskId,
    descending: false,
};

fn lowered(column: device::Column) -> sea_orm::sea_query::SimpleExpr {
    Expr::expr(Func::lower(Expr::col(column)))
}

fn filtered(query: &DeviceQuery) -> Select<device::Entity> {
    let mut select = device::Entity::find();
    if let Some(q) = query.q.as_deref().filter(|q| !q.is_empty()) {
        let needle = q.to_lowercase();
        let columns = [
            device::Column::RustdeskId,
            device::Column::Hostname,
            device::Column::Username,
            device::Column::Note,
        ];
        let any = columns.into_iter().fold(Condition::any(), |any, column| {
            any.add(lowered(column).like(contains_pattern(&needle)))
        });
        select = select.filter(any);
    }
    if let Some(os) = query.os.as_deref().filter(|os| !os.is_empty()) {
        let pattern = contains_pattern(&os.to_lowercase());
        select = select.filter(lowered(device::Column::Os).like(pattern));
    }
    if let Some(owner) = query.owner_user_id {
        select = select.filter(device::Column::OwnerUserId.eq(owner));
    }
    if let Some(group) = query.device_group_id {
        select = select.filter(device::Column::DeviceGroupId.eq(group));
    }
    if let Some(status) = query.status {
        select = select.filter(device::Column::Status.eq(status));
    }
    select
}

fn ordered(select: Select<device::Entity>, sort: DeviceSort) -> Select<device::Entity> {
    let order = match sort.descending {
        true => Order::Desc,
        false => Order::Asc,
    };
    select
        .order_by(sort.field.column(), order.clone())
        .order_by(device::Column::Id, order)
}

impl Inner {
    async fn page_in_db(
        &self,
        select: Select<device::Entity>,
        sort: DeviceSort,
        page: PageRequest,
        index: u64,
    ) -> Result<(u64, Vec<device::Model>), DeviceError> {
        let paginator = ordered(select, sort).paginate(&self.db, page.page_size);
        let total = paginator.num_items().await?;
        Ok((total, paginator.fetch_page(index).await?))
    }

    /// `devices.id` of the matching devices whose presence equals `online`, in order.
    async fn online_ids(
        &self,
        select: Select<device::Entity>,
        sort: DeviceSort,
        online: bool,
    ) -> Result<Vec<i64>, DeviceError> {
        let rows: Vec<(i64, String)> = ordered(select, sort)
            .select_only()
            .column(device::Column::Id)
            .column(device::Column::RustdeskId)
            .into_tuple()
            .all(&self.db)
            .await?;
        let matching = rows
            .into_iter()
            .filter(|(_, rustdesk_id)| self.presence.is_online(rustdesk_id) == online);
        Ok(matching.map(|(id, _)| id).collect())
    }

    /// The rows of the requested page of `ids`, in `ids` order, and the total.
    async fn page_of_ids(
        &self,
        ids: &[i64],
        page: PageRequest,
        index: u64,
    ) -> Result<(u64, Vec<device::Model>), DeviceError> {
        let size = usize::try_from(page.page_size).unwrap_or(usize::MAX);
        let skip = usize::try_from(index).unwrap_or(usize::MAX);
        let wanted: Vec<i64> = ids
            .iter()
            .copied()
            .skip(skip.saturating_mul(size))
            .take(size)
            .collect();
        let found = device::Entity::find()
            .filter(device::Column::Id.is_in(wanted.clone()))
            .all(&self.db)
            .await?;
        let mut by_id: HashMap<i64, device::Model> = found.into_iter().map(|m| (m.id, m)).collect();
        let rows = wanted.iter().filter_map(|id| by_id.remove(id)).collect();
        Ok((ids.len() as u64, rows))
    }

    /// `rows` with owner and group names and presence.
    pub(super) async fn summaries(
        &self,
        rows: Vec<device::Model>,
    ) -> Result<Vec<DeviceSummary>, DeviceError> {
        let owners = self.owner_names(&rows).await?;
        let groups = self.group_names(&rows).await?;
        Ok(rows
            .into_iter()
            .map(|row| DeviceSummary {
                owner_username: row.owner_user_id.and_then(|id| owners.get(&id).cloned()),
                group_name: row.device_group_id.and_then(|id| groups.get(&id).cloned()),
                presence: self.presence.get(&row.rustdesk_id),
                record: row.into(),
            })
            .collect())
    }

    async fn owner_names(
        &self,
        rows: &[device::Model],
    ) -> Result<HashMap<i64, String>, DeviceError> {
        let ids: Vec<i64> = rows.iter().filter_map(|r| r.owner_user_id).collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let named: Vec<(i64, String)> = user::Entity::find()
            .select_only()
            .column(user::Column::Id)
            .column(user::Column::Username)
            .filter(user::Column::Id.is_in(ids))
            .into_tuple()
            .all(&self.db)
            .await?;
        Ok(named.into_iter().collect())
    }

    async fn group_names(
        &self,
        rows: &[device::Model],
    ) -> Result<HashMap<i64, String>, DeviceError> {
        let ids: Vec<i64> = rows.iter().filter_map(|r| r.device_group_id).collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let named: Vec<(i64, String)> = device_group::Entity::find()
            .select_only()
            .column(device_group::Column::Id)
            .column(device_group::Column::Name)
            .filter(device_group::Column::Id.is_in(ids))
            .into_tuple()
            .all(&self.db)
            .await?;
        Ok(named.into_iter().collect())
    }
}
