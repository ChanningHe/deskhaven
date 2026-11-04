//! Books themselves: the personal book, shared book profiles, shared book management.

use std::collections::HashMap;

use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::{address_book_share, user};
use deskhaven_db::{is_foreign_key_violation, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, QueryTrait, SqlErr,
};

use super::access::{share_rules, shares_of};
use super::error::{AddressBookError, invalid};
use super::{AddressBookService, Book, PERSONAL_NAME, Profile, RESERVED_NAMES, Rule};
use crate::groups::member_group_ids;
use crate::pagination::{Page, PageRequest};
use crate::text::{self, NOTE_MAX_LEN};

const NAME_MAX_LEN: usize = 64;

impl AddressBookService {
    /// The personal book of `user_id`, created on first access. Concurrent first accesses
    /// return the same book.
    pub async fn personal(&self, user_id: i64) -> Result<Book, AddressBookError> {
        if let Some(book) = find_personal(&self.db, user_id).await? {
            return Ok(book.into());
        }
        let now = time::now();
        let row = address_book::ActiveModel {
            guid: Set(new_guid()),
            kind: Set(Kind::Personal),
            owner_user_id: Set(Some(user_id)),
            name: Set(PERSONAL_NAME.to_owned()),
            note: Set(String::new()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        // The partial unique index `uq_address_books_personal_owner` decides a race: the
        // loser's insert fails and it reads the winner's book. (`ON CONFLICT` cannot name a
        // partial index portably: SQLite wants the index predicate as a literal.)
        match row.insert(&self.db).await {
            Ok(created) => return Ok(created.into()),
            Err(err) if is_foreign_key_violation(&err) => {
                return Err(AddressBookError::UserNotFound);
            }
            Err(err) if is_unique_violation(&err) => {}
            Err(err) => return Err(err.into()),
        }
        let found = find_personal(&self.db, user_id).await?;
        found.map(Book::from).ok_or(AddressBookError::UserNotFound)
    }

    /// One page of the shared books `user_id` owns or is granted, ordered by name, with the
    /// user's effective rule on each.
    pub async fn shared_profiles(
        &self,
        user_id: i64,
        page: PageRequest,
    ) -> Result<Page<Profile>, AddressBookError> {
        let index = page.index().map_err(|p| invalid(p.field, p.reason))?;
        let groups = member_group_ids(&self.db, user_id).await?;
        let granted = address_book_share::Entity::find()
            .select_only()
            .column(address_book_share::Column::AddressBookId)
            .filter(shares_of(user_id, &groups))
            .into_query();
        let reachable = Condition::any()
            .add(address_book::Column::OwnerUserId.eq(user_id))
            .add(address_book::Column::Id.in_subquery(granted));
        let paginator = address_book::Entity::find()
            .filter(address_book::Column::Kind.eq(Kind::Shared))
            .filter(reachable)
            .order_by_asc(address_book::Column::Name)
            .order_by_asc(address_book::Column::Id)
            .paginate(&self.db, page.page_size);
        let total = paginator.num_items().await?;
        let books = paginator.fetch_page(index).await?;
        let items = self.profiles(user_id, books).await?;
        Ok(Page {
            items,
            total,
            page: page.page,
            page_size: page.page_size,
        })
    }

    async fn profiles(
        &self,
        user_id: i64,
        books: Vec<address_book::Model>,
    ) -> Result<Vec<Profile>, AddressBookError> {
        let ids: Vec<i64> = books.iter().map(|b| b.id).collect();
        let rules = share_rules(&self.db, user_id, &ids).await?;
        let owners = usernames(&self.db, books.iter().filter_map(|b| b.owner_user_id)).await?;
        let profiles = books.into_iter().filter_map(|book| {
            let rule = match book.owner_user_id == Some(user_id) {
                true => Rule::FullControl,
                // A share revoked between the two queries: no longer reachable.
                false => *rules.get(&book.id)?,
            };
            let owner = book.owner_user_id.and_then(|id| owners.get(&id).cloned());
            Some(Profile {
                book: book.into(),
                owner,
                rule,
            })
        });
        Ok(profiles.collect())
    }

    /// The one shared book named `name` (case-insensitive, surrounding blanks ignored), for
    /// administrative tools that address books by name. Several matches are
    /// [`AddressBookError::AmbiguousName`], none is [`AddressBookError::NotFound`].
    pub async fn find_shared_by_name(&self, name: &str) -> Result<Book, AddressBookError> {
        shared_by_name(&self.db, name).await
    }
}

/// [`AddressBookService::find_shared_by_name`] on `db` (e.g. a write transaction).
pub(crate) async fn shared_by_name(
    db: &impl ConnectionTrait,
    name: &str,
) -> Result<Book, AddressBookError> {
    let wanted = name.trim().to_lowercase();
    let ids: Vec<i64> = shared_names(db)
        .await?
        .into_iter()
        .filter(|(_, n)| n.to_lowercase() == wanted)
        .map(|(id, _)| id)
        .collect();
    let [id] = ids[..] else {
        return Err(match ids.is_empty() {
            true => AddressBookError::NotFound,
            false => AddressBookError::AmbiguousName,
        });
    };
    let found = address_book::Entity::find_by_id(id).one(db).await?;
    found.map(Book::from).ok_or(AddressBookError::NotFound)
}

/// Sets `updated_at` of `book` (after a change of its contents).
pub(crate) async fn touch(
    db: &impl ConnectionTrait,
    book: &address_book::Model,
) -> Result<(), AddressBookError> {
    let changes = address_book::ActiveModel {
        updated_at: Set(time::now()),
        ..Default::default()
    };
    address_book::Entity::update_many()
        .set(changes)
        .filter(address_book::Column::Id.eq(book.id))
        .exec(db)
        .await?;
    Ok(())
}

fn is_unique_violation(err: &DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

pub(super) fn new_guid() -> String {
    uuid::Uuid::now_v7().to_string()
}

async fn find_personal(
    db: &impl ConnectionTrait,
    user_id: i64,
) -> Result<Option<address_book::Model>, AddressBookError> {
    let found = address_book::Entity::find()
        .filter(address_book::Column::Kind.eq(Kind::Personal))
        .filter(address_book::Column::OwnerUserId.eq(user_id))
        .one(db)
        .await?;
    Ok(found)
}

pub(super) async fn usernames(
    db: &impl ConnectionTrait,
    ids: impl Iterator<Item = i64>,
) -> Result<HashMap<i64, String>, AddressBookError> {
    let mut ids: Vec<i64> = ids.collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .column(user::Column::Username)
        .filter(user::Column::Id.is_in(ids))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(rows.into_iter().collect())
}

async fn shared_names(db: &impl ConnectionTrait) -> Result<Vec<(i64, String)>, AddressBookError> {
    let rows = address_book::Entity::find()
        .select_only()
        .column(address_book::Column::Id)
        .column(address_book::Column::Name)
        .filter(address_book::Column::Kind.eq(Kind::Shared))
        .into_tuple::<(i64, String)>()
        .all(db)
        .await?;
    Ok(rows)
}

/// Refuses a reserved name or one another shared book (not `except`) already has.
pub(super) async fn ensure_name_free(
    db: &impl ConnectionTrait,
    name: &str,
    except: Option<i64>,
) -> Result<(), AddressBookError> {
    let wanted = name.to_lowercase();
    if RESERVED_NAMES.iter().any(|r| r.to_lowercase() == wanted) {
        return Err(AddressBookError::NameTaken);
    }
    let taken = shared_names(db)
        .await?
        .into_iter()
        .any(|(id, n)| Some(id) != except && n.to_lowercase() == wanted);
    match taken {
        true => Err(AddressBookError::NameTaken),
        false => Ok(()),
    }
}

/// Trimmed name: 1-64 characters, no control characters.
pub(super) fn validate_name(raw: &str) -> Result<String, AddressBookError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(invalid("name", "must not be blank"));
    }
    match text::problem(name, NAME_MAX_LEN, false) {
        Some(reason) => Err(invalid("name", reason)),
        None => Ok(name.to_owned()),
    }
}

pub(super) fn validate_note(raw: &str) -> Result<String, AddressBookError> {
    match text::problem(raw, NOTE_MAX_LEN, true) {
        Some(reason) => Err(invalid("note", reason)),
        None => Ok(raw.to_owned()),
    }
}
