//! Lock order (PostgreSQL), everywhere: subject row -> book rows -> share rows.
//!
//! A console write re-reads the acting account after it locked the book, the reverse order:
//! deleting an administrator while it writes a book it owns can deadlock, and PostgreSQL
//! aborts one side (a retryable error, nothing written). `set_share` refuses a share naming
//! the book's owner: the owner's deletion would wait on a share row held by a book writer.

use deskhaven_db::begin_write;
use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::address_book_share::{self, SubjectType};
use deskhaven_db::entity::{group, user};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{LockType, OnConflict};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};

use serde_json::Value;

use super::access::{Actor, authorize, is_steward};
use super::books::touch;
use super::error::{AddressBookError, invalid};
use super::{AddressBookService, Rule, snapshot};
use crate::console_audit::TxGuard;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Share {
    pub subject: Subject,
    pub rule: Rule,
}

impl AddressBookService {
    pub async fn set_share(
        &self,
        actor: Actor,
        guid: &str,
        subject: Subject,
        rule: Rule,
    ) -> Result<(), AddressBookError> {
        self.set_share_guarded(actor, guid, subject, rule, TxGuard::none())
            .await
    }

    pub async fn set_share_guarded(
        &self,
        actor: Actor,
        guid: &str,
        subject: Subject,
        rule: Rule,
        guard: TxGuard<Value>,
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        // Locked before the book (see the module docs); reported only after the access
        // check, so a missing subject reveals nothing to someone without access.
        let subject_exists = lock_subject(&tx, subject).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::FullControl, true).await?;
        if book.kind == Kind::Personal {
            return Err(AddressBookError::Personal);
        }
        let (subject_type, subject_id) = subject.parts();
        let current = address_book_share::Entity::find_by_id((book.id, subject_type, subject_id))
            .one(&tx)
            .await?;
        ensure_may_grant(actor, &book, current.as_ref().map(|c| c.rule), Some(rule))?;
        if !subject_exists {
            return Err(AddressBookError::SubjectNotFound);
        }
        if let (Subject::User(id), Some(owner)) = (subject, book.owner_user_id)
            && id == owner
        {
            return Err(invalid("subject", "the owner already has full control"));
        }
        upsert_share(&tx, book.id, subject, rule).await?;
        touch(&tx, &book).await?;
        let before = current.map(|c| snapshot::share(subject, c.rule));
        let after = snapshot::share(subject, rule);
        guard.check(&tx, before.as_ref(), Some(&after)).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn remove_share(
        &self,
        actor: Actor,
        guid: &str,
        subject: Subject,
    ) -> Result<(), AddressBookError> {
        self.remove_share_guarded(actor, guid, subject, TxGuard::none())
            .await
    }

    pub async fn remove_share_guarded(
        &self,
        actor: Actor,
        guid: &str,
        subject: Subject,
        guard: TxGuard<Value>,
    ) -> Result<(), AddressBookError> {
        let tx = begin_write(&self.db).await?;
        let (book, _) = authorize(&tx, actor, guid, Rule::FullControl, true).await?;
        if book.kind == Kind::Personal {
            return Err(AddressBookError::Personal);
        }
        let (subject_type, subject_id) = subject.parts();
        let key = (book.id, subject_type, subject_id);
        let current = address_book_share::Entity::find_by_id(key).one(&tx).await?;
        let Some(current) = current else {
            return Ok(());
        };
        ensure_may_grant(actor, &book, Some(current.rule), None)?;
        address_book_share::Entity::delete_by_id(key)
            .exec(&tx)
            .await?;
        touch(&tx, &book).await?;
        let before = snapshot::share(subject, current.rule);
        guard.check(&tx, Some(&before), None).await?;
        tx.commit().await?;
        Ok(())
    }

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

async fn upsert_share(
    tx: &DatabaseTransaction,
    book_id: i64,
    subject: Subject,
    rule: Rule,
) -> Result<(), DbErr> {
    let (subject_type, subject_id) = subject.parts();
    let row = address_book_share::ActiveModel {
        address_book_id: Set(book_id),
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
        .exec_without_returning(tx)
        .await?;
    Ok(())
}

fn ensure_may_grant(
    actor: Actor,
    book: &address_book::Model,
    current: Option<Rule>,
    new: Option<Rule>,
) -> Result<(), AddressBookError> {
    let involves_full = [current, new].contains(&Some(Rule::FullControl));
    match involves_full && !is_steward(actor, book) {
        true => Err(AddressBookError::Forbidden),
        false => Ok(()),
    }
}

/// PostgreSQL: `FOR KEY SHARE` until the transaction ends, so a deletion waits but other
/// updates of the row do not. SQLite's write transaction already excludes every other writer.
async fn lock_subject(db: &impl ConnectionTrait, subject: Subject) -> Result<bool, DbErr> {
    let found = match subject {
        Subject::User(id) => {
            user::Entity::find_by_id(id)
                .select_only()
                .column(user::Column::Id)
                .lock(LockType::KeyShare)
                .into_tuple::<i64>()
                .one(db)
                .await?
        }
        Subject::Group(id) => {
            group::Entity::find_by_id(id)
                .select_only()
                .column(group::Column::Id)
                .lock(LockType::KeyShare)
                .into_tuple::<i64>()
                .one(db)
                .await?
        }
    };
    Ok(found.is_some())
}

/// Locks every owned book in id order first (the lock order above). The personal book must
/// go before the user row: the database refuses to orphan one.
pub(crate) async fn forget_user(tx: &DatabaseTransaction, user_id: i64) -> Result<(), DbErr> {
    address_book::Entity::find()
        .select_only()
        .column(address_book::Column::Id)
        .filter(address_book::Column::OwnerUserId.eq(user_id))
        .order_by_asc(address_book::Column::Id)
        .lock_exclusive()
        .into_tuple::<i64>()
        .all(tx)
        .await?;
    address_book::Entity::delete_many()
        .filter(address_book::Column::OwnerUserId.eq(user_id))
        .filter(address_book::Column::Kind.eq(Kind::Personal))
        .exec(tx)
        .await?;
    delete_shares_of(tx, Subject::User(user_id)).await
}

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
