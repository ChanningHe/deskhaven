//! The console's view of shared books: a paged list with counts, one book, and its shares
//! with the names of their subjects. Personal books never appear here.

use std::collections::HashMap;

use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::{address_book_peer, address_book_share, address_book_tag, group};
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, Order, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};

use super::access::{Actor, authorize};
use super::books::usernames;
use super::error::{AddressBookError, invalid};
use super::{AddressBookService, Book, Rule, Share, Subject};
use crate::pagination::{Page, PageRequest};
use crate::text::contains_pattern;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookSortField {
    Name,
    CreatedAt,
    UpdatedAt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BookSort {
    pub field: BookSortField,
    pub descending: bool,
}

/// Filters and order of [`AddressBookService::admin_page`]; `q` matches a substring of the
/// name (case-insensitive, literal; ASCII only on SQLite).
#[derive(Clone, Debug, Default)]
pub struct BookQuery {
    pub q: Option<String>,
    pub sort: Option<BookSort>,
    pub page: PageRequest,
}

/// A shared book with its owner's name and the sizes of its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookSummary {
    pub book: Book,
    /// `None`: admin-managed.
    pub owner: Option<String>,
    pub peer_count: u64,
    pub tag_count: u64,
    pub share_count: u64,
}

/// A share with the name of its subject (`None`: the subject is gone and the share is about
/// to be, or was, cleaned up).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedShare {
    pub share: Share,
    pub subject_name: Option<String>,
}

impl AddressBookService {
    /// One page of shared books (ordered by `sort`, default name, then id in the same
    /// direction).
    pub async fn admin_page(
        &self,
        query: &BookQuery,
    ) -> Result<Page<BookSummary>, AddressBookError> {
        let index = query.page.index().map_err(|p| invalid(p.field, p.reason))?;
        let mut select =
            address_book::Entity::find().filter(address_book::Column::Kind.eq(Kind::Shared));
        if let Some(q) = query.q.as_deref().filter(|q| !q.is_empty()) {
            let name = Expr::expr(Func::lower(Expr::col(address_book::Column::Name)));
            select = select.filter(name.like(contains_pattern(&q.to_lowercase())));
        }
        let sort = query.sort.unwrap_or(BookSort {
            field: BookSortField::Name,
            descending: false,
        });
        let column = match sort.field {
            BookSortField::Name => address_book::Column::Name,
            BookSortField::CreatedAt => address_book::Column::CreatedAt,
            BookSortField::UpdatedAt => address_book::Column::UpdatedAt,
        };
        let order = match sort.descending {
            true => Order::Desc,
            false => Order::Asc,
        };
        let paginator = select
            .order_by(column, order.clone())
            .order_by(address_book::Column::Id, order)
            .paginate(&self.db, query.page.page_size);
        let total = paginator.num_items().await?;
        let books = paginator.fetch_page(index).await?;
        Ok(Page {
            items: self.summaries(books).await?,
            total,
            page: query.page.page,
            page_size: query.page.page_size,
        })
    }

    /// The shared book `guid`; a personal book is [`AddressBookError::NotFound`].
    pub async fn admin_book(&self, guid: &str) -> Result<BookSummary, AddressBookError> {
        let (book, _) = authorize(&self.db, Actor::Admin, guid, Rule::ReadOnly, false).await?;
        let mut summaries = self.summaries(vec![book]).await?;
        summaries.pop().ok_or(AddressBookError::NotFound)
    }

    /// Every share of the book with the names of its subjects; groups first, then users.
    pub async fn named_shares(
        &self,
        actor: Actor,
        guid: &str,
    ) -> Result<Vec<NamedShare>, AddressBookError> {
        let shares = self.shares(actor, guid).await?;
        let (mut users, mut groups) = (Vec::new(), Vec::new());
        for share in &shares {
            match share.subject {
                Subject::User(id) => users.push(id),
                Subject::Group(id) => groups.push(id),
            }
        }
        let user_names = usernames(&self.db, users.into_iter()).await?;
        let group_names = group_names(&self.db, groups).await?;
        let named = shares.into_iter().map(|share| {
            let subject_name = match share.subject {
                Subject::User(id) => user_names.get(&id).cloned(),
                Subject::Group(id) => group_names.get(&id).cloned(),
            };
            NamedShare {
                share,
                subject_name,
            }
        });
        Ok(named.collect())
    }

    async fn summaries(
        &self,
        books: Vec<address_book::Model>,
    ) -> Result<Vec<BookSummary>, AddressBookError> {
        let ids: Vec<i64> = books.iter().map(|b| b.id).collect();
        let owners = usernames(&self.db, books.iter().filter_map(|b| b.owner_user_id)).await?;
        let peers = counts::<address_book_peer::Entity>(
            &self.db,
            address_book_peer::Column::AddressBookId,
            &ids,
        )
        .await?;
        let tags = counts::<address_book_tag::Entity>(
            &self.db,
            address_book_tag::Column::AddressBookId,
            &ids,
        )
        .await?;
        let shares = counts::<address_book_share::Entity>(
            &self.db,
            address_book_share::Column::AddressBookId,
            &ids,
        )
        .await?;
        let summaries = books.into_iter().map(|book| BookSummary {
            owner: book.owner_user_id.and_then(|id| owners.get(&id).cloned()),
            peer_count: peers.get(&book.id).copied().unwrap_or(0),
            tag_count: tags.get(&book.id).copied().unwrap_or(0),
            share_count: shares.get(&book.id).copied().unwrap_or(0),
            book: book.into(),
        });
        Ok(summaries.collect())
    }
}

/// Rows per book id, for the books in `ids`.
async fn counts<E: EntityTrait>(
    db: &impl ConnectionTrait,
    column: E::Column,
    ids: &[i64],
) -> Result<HashMap<i64, u64>, DbErr> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, i64)> = E::find()
        .select_only()
        .column(column)
        .column_as(column.count(), "n")
        .filter(column.is_in(ids.iter().copied()))
        .group_by(column)
        .into_tuple()
        .all(db)
        .await?;
    let counted = rows
        .into_iter()
        .map(|(id, n)| (id, u64::try_from(n).unwrap_or(0)));
    Ok(counted.collect())
}

async fn group_names(
    db: &impl ConnectionTrait,
    mut ids: Vec<i64>,
) -> Result<HashMap<i64, String>, DbErr> {
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = group::Entity::find()
        .select_only()
        .column(group::Column::Id)
        .column(group::Column::Name)
        .filter(group::Column::Id.is_in(ids))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}
