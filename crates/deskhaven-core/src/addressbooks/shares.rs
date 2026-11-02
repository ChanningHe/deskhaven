//! Shares of shared books, and the cleanup when a share subject is deleted.
//!
//! Lock order (PostgreSQL): the subject row first, then the book row. Deleting a user locks
//! the user and then, through `ON DELETE SET NULL`, the books it owns; taking the locks in
//! the same order keeps a concurrent share change and user deletion from deadlocking.

use deskhaven_db::begin_write;
use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::address_book_share::{self, SubjectType};
use deskhaven_db::entity::{group, user};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use super::access::{Actor, authorize};
use super::books::touch;
use super::error::AddressBookError;
use super::{AddressBookService, Rule};

/// Whom a share grants a book to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    User(i64),
    Group(i64),
}

impl Subject {
    fn parts(self) -> (SubjectType, i64) {
        match self {
            Self::User(id) => (SubjectType::User, id),
            Self::Group(id) => (SubjectType::Group, id),
        }
    }

    fn from_parts(kind: SubjectType, id: i64) -> Self {
        match kind {
            SubjectType::User => Self::User(id),
            SubjectType::Group => Self::Group(id),
        }
    }
}

/// One grant of a shared book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Share {
    pub subject: Subject,
    pub rule: Rule,
}

impl AddressBookService {
    /// Grants (or changes) `rule` on a shared book to `subject`; needs full control.
    pub async fn set_share(
        &self,
        actor: Actor,
        guid: &str,
        subject: Subject,
        rule: Rule,
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        // Locked before the book (see the module docs); reported only after the access
        // check, so a missing subject reveals nothing to someone without access.
        let subject_exists = lock_subject(&tx, subject).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::FullControl, true).await?;
        if book.kind == Kind::Personal {
            return Err(AddressBookError::Personal);
        }
        if !subject_exists {
            return Err(AddressBookError::SubjectNotFound);
        }
        let (subject_type, subject_id) = subject.parts();
        let row = address_book_share::ActiveModel {
            address_book_id: Set(book.id),
            subject_type: Set(subject_type),
            subject_id: Set(subject_id),
            rule: Set(rule),
        };
        let on_conflict = OnConflict::columns([
            address_book_share::Column::AddressBookId,
            address_book_share::Column::SubjectType,
            address_book_share::Column::SubjectId,
        ])
        .update_column(address_book_share::Column::Rule)
        .to_owned();
        address_book_share::Entity::insert(row)
            .on_conflict(on_conflict)
            .exec_without_returning(&tx)
            .await?;
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Revokes the share of `subject`, if any; needs full control.
    pub async fn remove_share(
        &self,
        actor: Actor,
        guid: &str,
        subject: Subject,
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::FullControl, true).await?;
        if book.kind == Kind::Personal {
            return Err(AddressBookError::Personal);
        }
        let (subject_type, subject_id) = subject.parts();
        address_book_share::Entity::delete_by_id((book.id, subject_type, subject_id))
            .exec(&tx)
            .await?;
        touch(&tx, &book).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every share of a book, groups first, then users, each by id; needs full control.
    pub async fn shares(&self, actor: Actor, guid: &str) -> Result<Vec<Share>, AddressBookError> {
        let (book, _) = authorize(&self.db, actor, guid, Rule::FullControl, false).await?;
        let rows = address_book_share::Entity::find()
            .filter(address_book_share::Column::AddressBookId.eq(book.id))
            .order_by_asc(address_book_share::Column::SubjectType)
            .order_by_asc(address_book_share::Column::SubjectId)
            .all(&self.db)
            .await?;
        let shares = rows.into_iter().map(|row| Share {
            subject: Subject::from_parts(row.subject_type, row.subject_id),
            rule: row.rule,
        });
        Ok(shares.collect())
    }
}

/// Whether the subject row exists; locks it for the rest of the transaction.
async fn lock_subject(db: &impl ConnectionTrait, subject: Subject) -> Result<bool, DbErr> {
    let found = match subject {
        Subject::User(id) => {
            user::Entity::find_by_id(id)
                .select_only()
                .column(user::Column::Id)
                .lock_exclusive()
                .into_tuple::<i64>()
                .one(db)
                .await?
        }
        Subject::Group(id) => {
            group::Entity::find_by_id(id)
                .select_only()
                .column(group::Column::Id)
                .lock_exclusive()
                .into_tuple::<i64>()
                .one(db)
                .await?
        }
    };
    Ok(found.is_some())
}

/// Address book data of a user being deleted, inside the deleting transaction: the personal
/// book (peers and tags cascade) and every share naming the user. The personal book must go
/// before the user row: the database refuses to orphan one.
pub(crate) async fn forget_user(tx: &DatabaseTransaction, user_id: i64) -> Result<(), DbErr> {
    address_book::Entity::delete_many()
        .filter(address_book::Column::OwnerUserId.eq(user_id))
        .filter(address_book::Column::Kind.eq(Kind::Personal))
        .exec(tx)
        .await?;
    delete_shares_of(tx, Subject::User(user_id)).await
}

/// Every share naming a group being deleted, inside the deleting transaction.
pub(crate) async fn forget_group(tx: &DatabaseTransaction, group_id: i64) -> Result<(), DbErr> {
    delete_shares_of(tx, Subject::Group(group_id)).await
}

async fn delete_shares_of(tx: &DatabaseTransaction, subject: Subject) -> Result<(), DbErr> {
    let (subject_type, subject_id) = subject.parts();
    address_book_share::Entity::delete_many()
        .filter(address_book_share::Column::SubjectType.eq(subject_type))
        .filter(address_book_share::Column::SubjectId.eq(subject_id))
        .exec(tx)
        .await?;
    Ok(())
}
