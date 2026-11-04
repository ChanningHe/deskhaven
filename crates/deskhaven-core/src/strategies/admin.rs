//! Console reads: the paged strategy list with assignment counts, one strategy's summary,
//! and the targets a strategy is assigned to, with their names.

use std::collections::HashMap;

use deskhaven_db::entity::{device, device_group, group, strategy, strategy_assignment, user};
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, Order, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};

use super::model::{Strategy, Target};
use super::{StrategyError, StrategyService};
use crate::pagination::{Page, PageRequest};
use crate::text::contains_pattern;

/// Filters of [`StrategyService::page`]: `q` matches a substring of the name,
/// case-insensitively (ASCII on SQLite) and literally.
#[derive(Clone, Debug, Default)]
pub struct StrategyQuery {
    pub q: Option<String>,
    pub sort: Option<StrategySort>,
    pub page: PageRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrategySortField {
    Name,
    UpdatedAt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrategySort {
    pub field: StrategySortField,
    pub descending: bool,
}

/// A strategy with the number of targets it is assigned to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategySummary {
    pub strategy: Strategy,
    pub assignment_count: u64,
}

/// A target a strategy is assigned to. `name` is the user name, group or device group name,
/// or the device's RustDesk id; `None` for the global target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignedTarget {
    pub target: Target,
    pub name: Option<String>,
}

impl StrategyService {
    /// One page of strategies (ordered by `sort`, default name, then id the same way).
    pub async fn page(
        &self,
        query: &StrategyQuery,
    ) -> Result<Page<StrategySummary>, StrategyError> {
        let index = query.page.index().map_err(|p| StrategyError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let mut select = strategy::Entity::find();
        if let Some(q) = query.q.as_deref().filter(|q| !q.is_empty()) {
            let name = Expr::expr(Func::lower(Expr::col(strategy::Column::Name)));
            select = select.filter(name.like(contains_pattern(&q.to_lowercase())));
        }
        let sort = query.sort.unwrap_or(StrategySort {
            field: StrategySortField::Name,
            descending: false,
        });
        let column = match sort.field {
            StrategySortField::Name => strategy::Column::Name,
            StrategySortField::UpdatedAt => strategy::Column::UpdatedAt,
        };
        let order = match sort.descending {
            true => Order::Desc,
            false => Order::Asc,
        };
        let paginator = select
            .order_by(column, order.clone())
            .order_by(strategy::Column::Id, order)
            .paginate(&self.inner.db, query.page.page_size);
        let total = paginator.num_items().await?;
        let rows = paginator.fetch_page(index).await?;
        let counts = assignment_counts(&self.inner.db, rows.iter().map(|s| s.id)).await?;
        let items = rows
            .into_iter()
            .map(|row| StrategySummary {
                assignment_count: counts.get(&row.id).copied().unwrap_or(0),
                strategy: row.into(),
            })
            .collect();
        Ok(Page {
            items,
            total,
            page: query.page.page,
            page_size: query.page.page_size,
        })
    }

    pub async fn summary(&self, id: i64) -> Result<StrategySummary, StrategyError> {
        let strategy = self.get(id).await?;
        let counts = assignment_counts(&self.inner.db, [id]).await?;
        Ok(StrategySummary {
            assignment_count: counts.get(&id).copied().unwrap_or(0),
            strategy,
        })
    }

    /// The targets of strategy `id`, ordered like [`Target`] (global first, devices last).
    pub async fn assigned_targets(&self, id: i64) -> Result<Vec<AssignedTarget>, StrategyError> {
        let db = &self.inner.db;
        let exists = strategy::Entity::find_by_id(id).one(db).await?.is_some();
        if !exists {
            return Err(StrategyError::NotFound);
        }
        let rows = strategy_assignment::Entity::find()
            .filter(strategy_assignment::Column::StrategyId.eq(id))
            .all(db)
            .await?;
        let mut targets: Vec<Target> = rows
            .into_iter()
            .filter_map(|r| Target::from_row(r.target_type, r.target_id))
            .collect();
        targets.sort();
        let names = Names::load(db, &targets).await?;
        Ok(targets
            .into_iter()
            .map(|target| AssignedTarget {
                name: names.of(target),
                target,
            })
            .collect())
    }
}

async fn assignment_counts(
    db: &impl ConnectionTrait,
    ids: impl IntoIterator<Item = i64>,
) -> Result<HashMap<i64, u64>, DbErr> {
    let ids: Vec<i64> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, i64)> = strategy_assignment::Entity::find()
        .select_only()
        .column(strategy_assignment::Column::StrategyId)
        .column_as(strategy_assignment::Column::Id.count(), "assignments")
        .filter(strategy_assignment::Column::StrategyId.is_in(ids))
        .group_by(strategy_assignment::Column::StrategyId)
        .into_tuple()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, n)| (id, u64::try_from(n).unwrap_or(0)))
        .collect())
}

/// Names of the targets, one query per kind present.
#[derive(Default)]
struct Names {
    users: HashMap<i64, String>,
    groups: HashMap<i64, String>,
    device_groups: HashMap<i64, String>,
    devices: HashMap<i64, String>,
}

impl Names {
    async fn load(db: &impl ConnectionTrait, targets: &[Target]) -> Result<Self, DbErr> {
        let (mut users, mut groups, mut device_groups, mut devices) = Default::default();
        for target in targets {
            let ids: &mut Vec<i64> = match target {
                Target::Global => continue,
                Target::User(_) => &mut users,
                Target::Group(_) => &mut groups,
                Target::DeviceGroup(_) => &mut device_groups,
                Target::Device(_) => &mut devices,
            };
            ids.extend(target.id());
        }
        use device_group::Column as Dg;
        Ok(Self {
            users: named::<user::Entity>(db, user::Column::Id, user::Column::Username, users)
                .await?,
            groups: named::<group::Entity>(db, group::Column::Id, group::Column::Name, groups)
                .await?,
            device_groups: named::<device_group::Entity>(db, Dg::Id, Dg::Name, device_groups)
                .await?,
            devices: named::<device::Entity>(
                db,
                device::Column::Id,
                device::Column::RustdeskId,
                devices,
            )
            .await?,
        })
    }

    fn of(&self, target: Target) -> Option<String> {
        let (map, id) = match target {
            Target::Global => return None,
            Target::User(id) => (&self.users, id),
            Target::Group(id) => (&self.groups, id),
            Target::DeviceGroup(id) => (&self.device_groups, id),
            Target::Device(id) => (&self.devices, id),
        };
        map.get(&id).cloned()
    }
}

/// `id -> name` of the rows of `E` with the given ids.
async fn named<E: EntityTrait>(
    db: &impl ConnectionTrait,
    id_col: E::Column,
    name_col: E::Column,
    ids: Vec<i64>,
) -> Result<HashMap<i64, String>, DbErr> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, String)> = E::find()
        .select_only()
        .column(id_col)
        .column(name_col)
        .filter(id_col.is_in(ids))
        .into_tuple()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}
