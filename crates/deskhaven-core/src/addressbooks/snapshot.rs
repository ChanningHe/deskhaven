//! Audit snapshots of address book changes, the `before` / `after` a
//! [`TxGuard<Value>`](crate::console_audit::TxGuard) sees. Built here, where the secrets are
//! known: a peer's password and hash are never part of a snapshot, only whether the peer has
//! one (`had_password`; the audit redactor masks the `true`, so a row shows `"***"`).

use deskhaven_db::entity::address_book;
use deskhaven_db::entity::address_book_peer;
use deskhaven_db::entity::address_book_share::SubjectType;
use sea_orm::ActiveEnum;
use serde_json::{Value, json};

use super::{Rule, Subject, Tag};

/// A shared book's own fields.
pub(super) fn book(book: &address_book::Model) -> Value {
    json!({
        "book": {
            "guid": book.guid,
            "name": book.name,
            "note": book.note,
            "owner_user_id": book.owner_user_id,
        }
    })
}

/// A peer without its secrets.
pub(super) fn peer(row: &address_book_peer::Model) -> Value {
    json!({
        "peer": {
            "id": row.rustdesk_id,
            "alias": row.alias,
            "note": row.note,
            "username": row.username,
            "hostname": row.hostname,
            "platform": row.platform,
            "tags": serde_json::from_str::<Value>(&row.tags).unwrap_or(Value::Null),
            // `null` without one: the redactor keeps `null` and masks `true`.
            "had_password": row.password_enc.is_some().then_some(true),
        }
    })
}

/// The ids of several peers (a batch delete or import): `{"peer_ids": [..]}`.
pub(super) fn peer_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Value {
    json!({ "peer_ids": ids.collect::<Vec<_>>() })
}

pub(super) fn tag(tag: &Tag) -> Value {
    json!({ "tag": { "name": tag.name, "color": tag.color } })
}

/// Tag names (a batch delete): `{"tags": [..]}`.
pub(super) fn tag_names<'a>(names: impl Iterator<Item = &'a str>) -> Value {
    json!({ "tags": names.collect::<Vec<_>>() })
}

pub(super) fn share(subject: Subject, rule: Rule) -> Value {
    let (kind, id) = match subject {
        Subject::User(id) => (SubjectType::User, id),
        Subject::Group(id) => (SubjectType::Group, id),
    };
    json!({
        "share": {
            "subject_type": kind.to_value(),
            "subject_id": id,
            "rule": rule.to_value(),
        }
    })
}
