//! Who may do what with a book (06 §4.2).
//!
//! - A personal book belongs to its owner alone (full control); nobody else sees it, not
//!   even a console administrator through this service.
//! - A shared book gives its owner full control. Everyone else gets the highest rule among
//!   the shares naming them directly or naming a group they belong to. An unowned shared
//!   book (owner deleted) is admin-managed: users reach it only through shares.
//! - [`Actor::Admin`] (the console) has full control of every shared book.
//! - Rule 3 manages contents, tags, the book's name and note, and shares up to rule 2. Only
//!   the owner or the console ([`is_steward`]) deletes the book, grants rule 3, or changes
//!   or revokes an existing rule 3 grant: a manager cannot hand out its own power nor
//!   lock out another manager (06 §4.2).
//!
//! A book without any rule for the actor is reported as [`AddressBookError::NotFound`],
//! exactly like a book that does not exist.

use std::collections::HashMap;

use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::address_book_share::{self, Rule, SubjectType};
use sea_orm::{
    ActiveEnum, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter,
    QuerySelect,
};

use super::error::AddressBookError;
use crate::groups::member_group_ids;

/// Longer guids cannot exist (UUID strings); they are not looked up.
const GUID_MAX_LEN: usize = 64;

/// Who performs an address book operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// A console administrator: full control of every shared book, no access to personal
    /// books.
    Admin,
    /// A user (the client API): what ownership and shares grant.
    User(i64),
}

/// Whether `actor` is a steward of `book`: the console, or the owner of the book. Only a
/// steward deletes a book, grants rule 3 or changes and revokes a rule 3 grant; a rule 3
/// sharee manages the contents and the grants up to rule 2.
pub(crate) fn is_steward(actor: Actor, book: &address_book::Model) -> bool {
    match actor {
        Actor::Admin => true,
        Actor::User(id) => book.owner_user_id == Some(id),
    }
}

/// `a` grants at least what `b` grants.
pub fn covers(a: Rule, b: Rule) -> bool {
    a.to_value() >= b.to_value()
}

fn higher(a: Rule, b: Rule) -> Rule {
    if covers(a, b) { a } else { b }
}

/// The book `guid` if `actor` holds at least `needed` on it, with the rule it holds. Locked
/// for the rest of the transaction when `lock` (serializes writes to one book).
pub(crate) async fn authorize(
    db: &impl ConnectionTrait,
    actor: Actor,
    guid: &str,
    needed: Rule,
    lock: bool,
) -> Result<(address_book::Model, Rule), AddressBookError> {
    if guid.len() > GUID_MAX_LEN {
        return Err(AddressBookError::NotFound);
    }
    let mut select = address_book::Entity::find().filter(address_book::Column::Guid.eq(guid));
    if lock {
        select = select.lock_exclusive();
    }
    let book = select.one(db).await?.ok_or(AddressBookError::NotFound)?;
    let rule = rule_on(db, actor, &book)
        .await?
        .ok_or(AddressBookError::NotFound)?;
    if !covers(rule, needed) {
        return Err(AddressBookError::Forbidden);
    }
    Ok((book, rule))
}

/// The rule `actor` holds on `book`, if any.
pub(crate) async fn rule_on(
    db: &impl ConnectionTrait,
    actor: Actor,
    book: &address_book::Model,
) -> Result<Option<Rule>, DbErr> {
    let user_id = match (actor, book.kind) {
        (Actor::Admin, Kind::Shared) => return Ok(Some(Rule::FullControl)),
        (Actor::Admin, Kind::Personal) => return Ok(None),
        (Actor::User(user_id), _) if book.owner_user_id == Some(user_id) => {
            return Ok(Some(Rule::FullControl));
        }
        (Actor::User(_), Kind::Personal) => return Ok(None),
        (Actor::User(user_id), Kind::Shared) => user_id,
    };
    let rules = share_rules(db, user_id, &[book.id]).await?;
    Ok(rules.get(&book.id).copied())
}

/// The highest share rule of `user_id` (directly or through a group) per book, for the
/// books in `book_ids`. Ownership is not considered.
pub(crate) async fn share_rules(
    db: &impl ConnectionTrait,
    user_id: i64,
    book_ids: &[i64],
) -> Result<HashMap<i64, Rule>, DbErr> {
    if book_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let groups = member_group_ids(db, user_id).await?;
    let shares = address_book_share::Entity::find()
        .select_only()
        .column(address_book_share::Column::AddressBookId)
        .column(address_book_share::Column::Rule)
        .filter(address_book_share::Column::AddressBookId.is_in(book_ids.iter().copied()))
        .filter(shares_of(user_id, &groups))
        .into_tuple::<(i64, Rule)>()
        .all(db)
        .await?;
    let mut rules = HashMap::new();
    for (book_id, rule) in shares {
        rules
            .entry(book_id)
            .and_modify(|held| *held = higher(*held, rule))
            .or_insert(rule);
    }
    Ok(rules)
}

/// Share rows that name `user_id` or one of `groups`.
pub(crate) fn shares_of(user_id: i64, groups: &[i64]) -> Condition {
    let user = Condition::all()
        .add(address_book_share::Column::SubjectType.eq(SubjectType::User))
        .add(address_book_share::Column::SubjectId.eq(user_id));
    let mut any = Condition::any().add(user);
    if !groups.is_empty() {
        any = any.add(
            Condition::all()
                .add(address_book_share::Column::SubjectType.eq(SubjectType::Group))
                .add(address_book_share::Column::SubjectId.is_in(groups.iter().copied())),
        );
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_are_ordered_read_write_full() {
        assert!(covers(Rule::FullControl, Rule::ReadWrite));
        assert!(covers(Rule::ReadWrite, Rule::ReadWrite));
        assert!(!covers(Rule::ReadOnly, Rule::ReadWrite));
        assert_eq!(higher(Rule::ReadOnly, Rule::FullControl), Rule::FullControl);
        assert_eq!(higher(Rule::ReadWrite, Rule::ReadOnly), Rule::ReadWrite);
    }
}
