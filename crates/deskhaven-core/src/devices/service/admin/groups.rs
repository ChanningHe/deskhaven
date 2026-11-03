//! Device groups for the console: paged list with device counts, detail, create, update and
//! delete. Groups with `managed_by = config` are read-only here
//! ([`DeviceError::ManagedByConfig`]). Deleting a group ungroups its devices (foreign key
//! `SET NULL`) and drops its strategy assignment.

use std::collections::HashMap;

use deskhaven_db::begin_write;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::{device, device_group};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, Order, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, SqlErr,
};

use crate::console_audit::TxGuard;
use crate::devices::DeviceError;
use crate::devices::service::{DEVICE_GROUP_NAME_MAX_LEN, DeviceService};
use crate::pagination::{Page, PageRequest};
use crate::strategies;
use crate::text::{self, NOTE_MAX_LEN, contains_pattern};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceGroup {
    pub id: i64,
    pub name: String,
    pub note: String,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<device_group::Model> for DeviceGroup {
    fn from(m: device_group::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            note: m.note,
            managed_by: m.managed_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Fields to change; `None` leaves a field as it is.
#[derive(Clone, Debug, Default)]
pub struct DeviceGroupUpdate {
    pub name: Option<String>,
    pub note: Option<String>,
}

/// Filters of [`DeviceService::group_page`]: `q` matches a substring of the name,
/// case-insensitively (ASCII on SQLite) and literally.
#[derive(Clone, Debug, Default)]
pub struct DeviceGroupQuery {
    pub q: Option<String>,
    pub sort: Option<DeviceGroupSort>,
    pub page: PageRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceGroupSortField {
    Name,
    CreatedAt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceGroupSort {
    pub field: DeviceGroupSortField,
    pub descending: bool,
}

/// A device group with the number of devices in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceGroupSummary {
    pub group: DeviceGroup,
    pub device_count: u64,
}

impl DeviceService {
    /// One page of groups (ordered by `sort`, default name, then id in the same direction).
    pub async fn group_page(
        &self,
        query: &DeviceGroupQuery,
    ) -> Result<Page<DeviceGroupSummary>, DeviceError> {
        let index = query.page.index().map_err(|p| DeviceError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let mut select = device_group::Entity::find();
        if let Some(q) = query.q.as_deref().filter(|q| !q.is_empty()) {
            let name = Expr::expr(Func::lower(Expr::col(device_group::Column::Name)));
            select = select.filter(name.like(contains_pattern(&q.to_lowercase())));
        }
        let sort = query.sort.unwrap_or(DeviceGroupSort {
            field: DeviceGroupSortField::Name,
            descending: false,
        });
        let column = match sort.field {
            DeviceGroupSortField::Name => device_group::Column::Name,
            DeviceGroupSortField::CreatedAt => device_group::Column::CreatedAt,
        };
        let order = match sort.descending {
            true => Order::Desc,
            false => Order::Asc,
        };
        let paginator = select
            .order_by(column, order.clone())
            .order_by(device_group::Column::Id, order)
            .paginate(&self.inner.db, query.page.page_size);
        let total = paginator.num_items().await?;
        let groups = paginator.fetch_page(index).await?;
        let counts = self.device_counts(groups.iter().map(|g| g.id)).await?;
        let items = groups
            .into_iter()
            .map(|g| DeviceGroupSummary {
                device_count: counts.get(&g.id).copied().unwrap_or(0),
                group: g.into(),
            })
            .collect();
        Ok(Page {
            items,
            total,
            page: query.page.page,
            page_size: query.page.page_size,
        })
    }

    /// The group with its number of devices.
    pub async fn group_summary(&self, id: i64) -> Result<DeviceGroupSummary, DeviceError> {
        let found = device_group::Entity::find_by_id(id)
            .one(&self.inner.db)
            .await?;
        let group = found.ok_or(DeviceError::GroupNotFound)?;
        let counts = self.device_counts([id]).await?;
        Ok(DeviceGroupSummary {
            device_count: counts.get(&id).copied().unwrap_or(0),
            group: group.into(),
        })
    }

    async fn device_counts(
        &self,
        ids: impl IntoIterator<Item = i64>,
    ) -> Result<HashMap<i64, u64>, DeviceError> {
        let ids: Vec<i64> = ids.into_iter().collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(i64, i64)> = device::Entity::find()
            .select_only()
            .column(device::Column::DeviceGroupId)
            .column_as(device::Column::Id.count(), "devices")
            .filter(device::Column::DeviceGroupId.is_in(ids))
            .group_by(device::Column::DeviceGroupId)
            .into_tuple()
            .all(&self.inner.db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(id, n)| (id, u64::try_from(n).unwrap_or(0)))
            .collect())
    }

    /// Creates a group managed in the console; `guard` decides on the new group (no
    /// `before`). Names are unique (exact match).
    pub async fn create_group_guarded(
        &self,
        name: &str,
        note: &str,
        guard: TxGuard<DeviceGroup>,
    ) -> Result<DeviceGroup, DeviceError> {
        let name = validate_group_name(name)?;
        let note = validate_group_note(note)?;
        self.detached(move |inner| async move {
            let now = time::now();
            let row = device_group::ActiveModel {
                name: Set(name),
                note: Set(note),
                managed_by: Set(ManagedBy::Ui),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            };
            let tx = begin_write(&inner.db).await?;
            let created: DeviceGroup = row.insert(&tx).await.map_err(name_conflict)?.into();
            guard.check(&tx, None, Some(&created)).await?;
            tx.commit().await?;
            Ok(created)
        })
        .await
    }

    /// Renames the group or changes its note, with `guard` deciding on it before and after.
    /// An empty update changes and records nothing.
    pub async fn update_group_guarded(
        &self,
        id: i64,
        update: DeviceGroupUpdate,
        guard: TxGuard<DeviceGroup>,
    ) -> Result<DeviceGroup, DeviceError> {
        let name = update
            .name
            .as_deref()
            .map(validate_group_name)
            .transpose()?;
        let note = update
            .note
            .as_deref()
            .map(validate_group_note)
            .transpose()?;
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let current = writable_group(&tx, id).await?;
            if name.is_none() && note.is_none() {
                return Ok(DeviceGroup::from(current));
            }
            let before = DeviceGroup::from(current.clone());
            let mut changes: device_group::ActiveModel = current.into();
            if let Some(name) = name {
                changes.name = Set(name);
            }
            if let Some(note) = note {
                changes.note = Set(note);
            }
            changes.updated_at = Set(time::now());
            let updated: DeviceGroup = changes.update(&tx).await.map_err(name_conflict)?.into();
            guard.check(&tx, Some(&before), Some(&updated)).await?;
            tx.commit().await?;
            Ok(updated)
        })
        .await
    }

    /// Deletes the group and its strategy assignment, with `guard` deciding on the group
    /// (no `after`); its devices become ungrouped.
    pub async fn delete_group_guarded(
        &self,
        id: i64,
        guard: TxGuard<DeviceGroup>,
    ) -> Result<(), DeviceError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let current = writable_group(&tx, id).await?;
            guard
                .check(&tx, Some(&DeviceGroup::from(current)), None)
                .await?;
            strategies::forget_device_group(&tx, id).await?;
            device_group::Entity::delete_by_id(id).exec(&tx).await?;
            tx.commit().await?;
            inner.strategies.cache().invalidate();
            Ok(())
        })
        .await
    }
}

/// The group, locked for the rest of the transaction, if the console may change it.
async fn writable_group(
    db: &impl ConnectionTrait,
    id: i64,
) -> Result<device_group::Model, DeviceError> {
    let found = device_group::Entity::find_by_id(id)
        .lock_exclusive()
        .one(db)
        .await?;
    let group = found.ok_or(DeviceError::GroupNotFound)?;
    match group.managed_by {
        ManagedBy::Config => Err(DeviceError::ManagedByConfig),
        ManagedBy::Ui => Ok(group),
    }
}

/// Trimmed name: 1-[`DEVICE_GROUP_NAME_MAX_LEN`] characters, no control characters.
fn validate_group_name(raw: &str) -> Result<String, DeviceError> {
    let name = raw.trim();
    let problem = match name.is_empty() {
        true => Some("must not be blank".to_owned()),
        false => text::problem(name, DEVICE_GROUP_NAME_MAX_LEN, false),
    };
    match problem {
        Some(reason) => Err(DeviceError::Invalid {
            field: "name",
            reason,
        }),
        None => Ok(name.to_owned()),
    }
}

fn validate_group_note(raw: &str) -> Result<String, DeviceError> {
    match text::problem(raw, NOTE_MAX_LEN, true) {
        Some(reason) => Err(DeviceError::Invalid {
            field: "note",
            reason,
        }),
        None => Ok(raw.to_owned()),
    }
}

fn name_conflict(err: DbErr) -> DeviceError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => DeviceError::Conflict { field: "name" },
        _ => DeviceError::Database(err),
    }
}
