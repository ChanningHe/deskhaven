//! A book the actor holds no rule on is `NotFound`, exactly like a missing book. Only a
//! steward (owner or console) deletes a book or grants, changes or revokes rule 3: a manager
//! cannot hand out its own power nor lock out another manager.

use std::collections::HashMap;

use deskhaven_db::entity::address_book::{self, Kind};
use deskhaven_db::entity::address_book_share::{self, Rule, SubjectType};
use sea_orm::{
    ActiveEnum, ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter,
    QuerySelect,
};

use super::error::AddressBookError;
use crate::groups::member_group_ids;

const GUID_MAX_LEN: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    Admin,
    User(i64),
}

pub(crate) fn is_steward(actor: Actor, book: &address_book::Model) -> bool {
    match actor {
        Actor::Admin => true,
        Actor::User(id) => book.owner_user_id == Some(id),
    }
}

pub fn covers(a: Rule, b: Rule) -> bool {
    a.to_value() >= b.to_value()
}

fn higher(a: Rule, b: Rule) -> Rule {
    if covers(a, b) { a } else { b }
}

/// `lock` holds the book row for the rest of the transaction, serializing writes to one book.
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
