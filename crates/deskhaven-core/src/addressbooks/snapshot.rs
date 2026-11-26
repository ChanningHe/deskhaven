//! Built here, where the secrets are known: a peer's password and hash never enter a snapshot,
//! only `had_password`.

use deskhaven_db::entity::address_book;
use deskhaven_db::entity::address_book_peer;
use deskhaven_db::entity::address_book_share::SubjectType;
use sea_orm::ActiveEnum;
use serde_json::{Value, json};

use super::{Rule, Subject, Tag};

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

pub(super) fn peer_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Value {
    json!({ "peer_ids": ids.collect::<Vec<_>>() })
}

pub(super) fn tag(tag: &Tag) -> Value {
    json!({ "tag": { "name": tag.name, "color": tag.color } })
}

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
