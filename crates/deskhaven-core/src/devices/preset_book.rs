//! `preset-address-book-*` (only with `devices.accept_presets = all`, see
//! [`presets`](super::presets)): a peer entry for the device in the shared book of that name
//! (case-insensitive, unique).
//!
//! The entry is written **as the device's owner** (`Actor::User`, the owner the presets just
//! set or the one it already had), through the normal rule checks: a device without an
//! owner, or whose owner lacks read-write access (rule 2) to the book, gets no entry. So a
//! device can only add itself where its owner could have added it by hand.
//!
//! An existing entry is never replaced (and then no tag is created). The tag is created in
//! the book only after the entry was added, and not beyond the book's tag limit: then the
//! entry stays without it. Every skip is logged.

use deskhaven_db::entity::device;
use serde_json::{Map, Value, json};

use super::presets::{AddressBookPreset, log_failure};
use crate::addressbooks::{Actor, AddressBookError, AddressBookService, PeerPatch, Tag};

/// Colour of a tag a preset creates: the client's `Colors.grey` (ARGB), its neutral choice.
pub const PRESET_TAG_COLOR: i64 = 0xFF9E_9E9E;

const PRESET: &str = "preset-address-book-name";

pub(super) async fn apply(
    books: &AddressBookService,
    device: &device::Model,
    preset: &AddressBookPreset,
) {
    let id = device.rustdesk_id.as_str();
    let Some(owner) = device.owner_user_id else {
        tracing::info!(rustdesk_id = id, "{PRESET} needs an owner; ignored");
        return;
    };
    let actor = Actor::User(owner);
    let guid = match books.find_shared_by_name(&preset.name).await {
        Ok(book) => book.guid,
        Err(AddressBookError::NotFound | AddressBookError::AmbiguousName) => {
            tracing::info!(
                rustdesk_id = id,
                "{PRESET} names no single shared address book; ignored"
            );
            return;
        }
        Err(err) => return log_failure(id, PRESET, &err),
    };
    if add_entry(books, actor, &guid, device, preset).await
        && let Some(tag) = &preset.tag
    {
        tag_entry(books, actor, &guid, id, tag).await;
    }
}

/// `true` if the entry was added.
async fn add_entry(
    books: &AddressBookService,
    actor: Actor,
    guid: &str,
    device: &device::Model,
    preset: &AddressBookPreset,
) -> bool {
    let id = device.rustdesk_id.as_str();
    let patch = match PeerPatch::from_json(peer_json(device, preset)) {
        Ok(patch) => patch,
        Err(err) => {
            log_failure(id, PRESET, &err);
            return false;
        }
    };
    match books.add_peer(actor, guid, patch).await {
        Ok(()) => true,
        Err(AddressBookError::PeerExists) => {
            tracing::info!(
                rustdesk_id = id,
                "{PRESET}: the book already lists the device"
            );
            false
        }
        Err(AddressBookError::NotFound | AddressBookError::Forbidden) => {
            tracing::info!(
                rustdesk_id = id,
                "{PRESET}: the owner cannot write to that book; ignored"
            );
            false
        }
        Err(err) => {
            log_failure(id, PRESET, &err);
            false
        }
    }
}

/// Creates `tag` in the book if missing (and room is left), then puts it on the entry.
async fn tag_entry(books: &AddressBookService, actor: Actor, guid: &str, id: &str, tag: &str) {
    let new = Tag {
        name: tag.to_owned(),
        color: PRESET_TAG_COLOR,
    };
    if let Err(err) = books.add_tag(actor, guid, new).await {
        return log_failure(id, "preset-address-book-tag", &err);
    }
    let patch = PeerPatch::from_json(json!({ "id": id, "tags": [tag] }));
    let tagged = match patch {
        Ok(patch) => books.update_peer(actor, guid, patch).await,
        Err(err) => Err(err),
    };
    if let Err(err) = tagged {
        log_failure(id, "preset-address-book-tag", &err);
    }
}

/// The peer entry as the client would send it (validated by [`PeerPatch::from_json`]).
fn peer_json(device: &device::Model, preset: &AddressBookPreset) -> Value {
    let mut peer = Map::new();
    peer.insert("id".to_owned(), json!(device.rustdesk_id));
    let optional = [
        ("alias", preset.alias.as_deref()),
        ("note", preset.note.as_deref()),
        ("password", preset.password.as_deref().map(String::as_str)),
        ("hostname", device.hostname.as_deref()),
        ("username", device.username.as_deref()),
    ];
    for (key, value) in optional {
        if let Some(value) = value {
            peer.insert(key.to_owned(), json!(value));
        }
    }
    Value::Object(peer)
}
