//! Paginated group listing with member counts (console).

use std::collections::HashMap;

use deskhaven_db::entity::{group, group_member};
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect,
};

use super::{Group, GroupError, GroupService};
use crate::pagination::{Page, PageRequest};
use crate::text::contains_pattern;

/// Filters of [`GroupService::page`]: `q` matches a substring of the name,
/// case-insensitively (ASCII on SQLite) and literally.
#[derive(Clone, Debug, Default)]
pub struct GroupQuery {
    pub q: Option<String>,
    pub sort: Option<GroupSort>,
    pub page: PageRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupSortField {
    Name,
    CreatedAt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupSort {
    pub field: GroupSortField,
    pub descending: bool,
}

/// A group with its number of members.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupSummary {
    pub group: Group,
    pub member_count: u64,
}

impl GroupService {
    /// One page of groups (ordered by `sort`, default name, then id in the same direction)
    /// with member counts.
    pub async fn page(&self, query: &GroupQuery) -> Result<Page<GroupSummary>, GroupError> {
        let index = query.page.index().map_err(|p| GroupError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let mut select = group::Entity::find();
        if let Some(q) = query.q.as_deref().filter(|q| !q.is_empty()) {
            let name = Expr::expr(Func::lower(Expr::col(group::Column::Name)));
            select = select.filter(name.like(contains_pattern(&q.to_lowercase())));
        }
        let sort = query.sort.unwrap_or(GroupSort {
            field: GroupSortField::Name,
            descending: false,
        });
        let column = match sort.field {
            GroupSortField::Name => group::Column::Name,
            GroupSortField::CreatedAt => group::Column::CreatedAt,
        };
        let order = if sort.descending {
            Order::Desc
        } else {
            Order::Asc
        };
        let paginator = select
            .order_by(column, order.clone())
            .order_by(group::Column::Id, order)
            .paginate(&self.db, query.page.page_size);
        let total = paginator.num_items().await?;
        let groups = paginator.fetch_page(index).await?;
        let counts = self.member_counts(groups.iter().map(|g| g.id)).await?;
        let items = groups
            .into_iter()
            .map(|g| GroupSummary {
                member_count: counts.get(&g.id).copied().unwrap_or(0),
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

    /// The group with its number of members.
    pub async fn summary(&self, id: i64) -> Result<GroupSummary, GroupError> {
        let group = self.get(id).await?;
        let counts = self.member_counts([id]).await?;
        Ok(GroupSummary {
            member_count: counts.get(&id).copied().unwrap_or(0),
            group,
        })
    }

    async fn member_counts(
        &self,
        ids: impl IntoIterator<Item = i64>,
    ) -> Result<HashMap<i64, u64>, GroupError> {
        let ids: Vec<i64> = ids.into_iter().collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(i64, i64)> = group_member::Entity::find()
            .select_only()
            .column(group_member::Column::GroupId)
            .column_as(group_member::Column::UserId.count(), "members")
            .filter(group_member::Column::GroupId.is_in(ids))
            .group_by(group_member::Column::GroupId)
            .into_tuple()
            .all(&self.db)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(id, n)| (id, u64::try_from(n).unwrap_or(0)))
            .collect())
    }
}
