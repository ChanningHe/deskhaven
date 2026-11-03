//! The device's entry in a shared address book, by book name (case-insensitive, unique):
//! `preset-address-book-*` (only with `devices.accept_presets = all`, see
//! [`presets`](super::presets)) and `--assign --address_book_name` (an API token).
//!
//! A preset entry is written **as the device's owner** (`Actor::User`, the owner the presets
//! just set or the one it already had), through the normal rule checks: a device without an
//! owner, or whose owner lacks read-write access (rule 2) to the book, gets no entry. So a
//! device can only add itself where its owner could have added it by hand. A preset never
//! replaces an existing entry (and then creates no tag). The tag is created in the book only
//! after the entry was added, and not beyond the book's tag limit: then the entry stays
//! without it.
//!
//! An assignment writes as `Actor::Admin`, inside its own transaction
//! (`AddressBookService::place_entry_in`): it adds the entry or updates an existing one
//! (given fields replaced, the tag added), and any failure rolls the whole assignment back.

use deskhaven_db::entity::device;
use serde_json::{Map, Value, json};

use super::presets::{AddressBookPreset, log_failure};
use crate::addressbooks::{Actor, AddressBookError, AddressBookService, PeerPatch, Tag};

/// Colour of a tag a preset creates: the client's `Colors.grey` (ARGB), its neutral choice.
pub const PRESET_TAG_COLOR: i64 = 0xFF9E_9E9E;

const PRESET: &str = "preset-address-book-name";

/// The presets' entry: every failure is logged and skipped.
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
    match add_entry(books, Actor::User(owner), &guid, device, preset).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(
                rustdesk_id = id,
                "{PRESET}: the book already lists the device"
            );
        }
        Err(AddressBookError::NotFound | AddressBookError::Forbidden) => {
            tracing::info!(
                rustdesk_id = id,
                "{PRESET}: the owner cannot write to that book; ignored"
            );
        }
        Err(err) => log_failure(id, PRESET, &err),
    }
}

/// Adds the device's entry to book `guid` as `actor`, then tags it; `Ok(false)` when the
/// book already lists the device (left as it is, no tag created).
pub(super) async fn add_entry(
    books: &AddressBookService,
    actor: Actor,
    guid: &str,
    device: &device::Model,
    preset: &AddressBookPreset,
) -> Result<bool, AddressBookError> {
    let patch = PeerPatch::from_json(peer_json(device, preset))?;
    match books.add_peer(actor, guid, patch).await {
        Ok(()) => {}
        Err(AddressBookError::PeerExists) => return Ok(false),
        Err(err) => return Err(err),
    }
    if let Some(tag) = &preset.tag {
        tag_entry(books, actor, guid, &device.rustdesk_id, tag).await?;
    }
    Ok(true)
}

/// The entry's own fields (alias, note, password, tag) are acceptable, checked before
/// anything is written.
pub(super) fn check(rustdesk_id: &str, preset: &AddressBookPreset) -> Result<(), AddressBookError> {
    let mut peer = entry_fields(rustdesk_id, preset);
    if let Some(tag) = &preset.tag {
        peer.insert("tags".to_owned(), json!([tag]));
    }
    PeerPatch::from_json(Value::Object(peer)).map(drop)
}

/// An assignment's entry for device `rustdesk_id` (written as an administrator, in the
/// assignment's transaction): the preset's fields, the tag to add, and the entry's hostname /
/// username only when the assignment names the device (`device_name` / `device_username`),
/// so an existing entry keeps the names written into it.
pub(super) fn assign_patch(
    rustdesk_id: &str,
    preset: &AddressBookPreset,
    hostname: Option<&str>,
    username: Option<&str>,
) -> Result<PeerPatch, AddressBookError> {
    let mut peer = entry_fields(rustdesk_id, preset);
    for (key, value) in [("hostname", hostname), ("username", username)] {
        if let Some(value) = value {
            peer.insert(key.to_owned(), json!(value));
        }
    }
    if let Some(tag) = &preset.tag {
        peer.insert("tags".to_owned(), json!([tag]));
    }
    PeerPatch::from_json(Value::Object(peer))
}

/// The tag a preset or assignment creates in the book.
pub(super) fn new_tag(preset: &AddressBookPreset) -> Option<Tag> {
    preset.tag.as_ref().map(|name| Tag {
        name: name.clone(),
        color: PRESET_TAG_COLOR,
    })
}

/// Creates `tag` in the book if missing (and room is left), then puts it on the entry.
async fn tag_entry(
    books: &AddressBookService,
    actor: Actor,
    guid: &str,
    id: &str,
    tag: &str,
) -> Result<(), AddressBookError> {
    let new = Tag {
        name: tag.to_owned(),
        color: PRESET_TAG_COLOR,
    };
    books.add_tag(actor, guid, new).await?;
    let patch = PeerPatch::from_json(json!({ "id": id, "tags": [tag] }))?;
    books.update_peer(actor, guid, patch).await
}

/// The peer entry as the client would send it (validated by [`PeerPatch::from_json`]).
fn peer_json(device: &device::Model, preset: &AddressBookPreset) -> Value {
    let mut peer = entry_fields(&device.rustdesk_id, preset);
    let optional = [
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

fn entry_fields(rustdesk_id: &str, preset: &AddressBookPreset) -> Map<String, Value> {
    let mut peer = Map::new();
    peer.insert("id".to_owned(), json!(rustdesk_id));
    let optional = [
        ("alias", preset.alias.as_deref()),
        ("note", preset.note.as_deref()),
        ("password", preset.password.as_deref().map(String::as_str)),
    ];
    for (key, value) in optional {
        if let Some(value) = value {
            peer.insert(key.to_owned(), json!(value));
        }
    }
    peer
}
