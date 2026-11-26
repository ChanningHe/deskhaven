use sea_orm::sea_query::{Condition, Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, Order, PaginatorTrait, QueryFilter,
    QueryOrder, Select,
};

use super::{AuditQuery, AuditQueryError};
use crate::pagination::{Page, PageRequest};
use crate::text::contains_pattern;

#[derive(Clone, Copy, Debug, Default)]
pub struct Listing {
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub oldest_first: bool,
    pub page: PageRequest,
}

impl Listing {
    fn order(&self) -> Order {
        match self.oldest_first {
            true => Order::Asc,
            false => Order::Desc,
        }
    }

    pub(super) async fn run<E, C>(
        &self,
        db: &C,
        select: Select<E>,
        (created_at, id): (E::Column, E::Column),
    ) -> Result<Page<E::Model>, AuditQueryError>
    where
        E: EntityTrait,
        E::Model: Sync,
        C: ConnectionTrait,
    {
        let index = self.page.index().map_err(|p| AuditQueryError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let mut select = select;
        if let Some(from) = self.from {
            select = select.filter(created_at.gte(from));
        }
        if let Some(to) = self.to {
            select = select.filter(created_at.lt(to));
        }
        let paginator = select
            .order_by(created_at, self.order())
            .order_by(id, self.order())
            .paginate(db, self.page.page_size);
        let total = paginator.num_items().await?;
        let items = paginator.fetch_page(index).await?;
        Ok(Page {
            items,
            total,
            page: self.page.page,
            page_size: self.page.page_size,
        })
    }
}

impl<T> Page<T> {
    pub(super) fn with_items<U>(self, items: Vec<U>) -> Page<U> {
        Page {
            items,
            total: self.total,
            page: self.page,
            page_size: self.page_size,
        }
    }
}

impl AuditQuery {
    /// SQLite's `LIKE` is case-insensitive by itself, and `lower()` around the column would halve
    /// this scan's speed; PostgreSQL's is not.
    pub(super) fn contains<C: ColumnTrait>(&self, columns: &[C], needle: &str) -> Condition {
        let fold = self.db.get_database_backend() != DbBackend::Sqlite;
        let pattern = match fold {
            true => needle.to_lowercase(),
            false => needle.to_owned(),
        };
        columns.iter().fold(Condition::any(), |any, &column| {
            let column = match fold {
                true => Expr::expr(Func::lower(Expr::col(column))),
                false => Expr::col(column),
            };
            any.add(column.like(contains_pattern(&pattern)))
        })
    }
}

pub(super) fn text(q: Option<&str>) -> Option<&str> {
    q.map(str::trim).filter(|q| !q.is_empty())
}
