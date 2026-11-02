//! Deployment presets: the `preset-*` fields a client adds to its sysinfo (03 §2.2;
//! `src/hbbs_http/sync.rs:146-188`), set by the custom client's built-in options or its
//! config, to place a freshly installed device without an administrator.
//!
//! **When**: only with the first sysinfo a device row ever stores (`sysinfo_hash` was
//! `NULL`), i.e. when the device is new to deskhaven. Later uploads never apply them again,
//! so an administrator's change (console, `--assign`) is never undone by the device, and a
//! device that is already placed cannot be moved by whoever learns its id and uuid (a weak
//! secret, 06 §2). Presets that could not be applied then (unknown name) stay unapplied;
//! place the device by `--assign` or in the console instead.
//!
//! **What**, each only where nothing is set yet (never replacing a value):
//!
//! | field | effect |
//! |---|---|
//! | `preset-user-name` | owner = the user with that username (case-insensitive) |
//! | `preset-device-group-name` | device group = the group with exactly that name |
//! | `preset-note` | device note |
//! | `preset-strategy-name` | device-level strategy assignment = the strategy with that name |
//! | `preset-address-book-name` (+ `-tag`, `-alias`, `-password`, `-note`) | a peer entry for the device in the shared book of that name (case-insensitive, unique), unless the book already lists it; the tag is created in the book if missing |
//!
//! Unknown or ambiguous names, invalid values and full books are skipped (logged); they never
//! fail the sysinfo upload. Owner, device group and note are written in the sysinfo
//! transaction; the strategy and the address book entry right after it commits (best effort:
//! a crash in between loses them, like an unknown name would).

use deskhaven_db::entity::{device, device_group, user};
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use crate::addressbooks::{Actor, AddressBookError, AddressBookService, PeerPatch, Tag};
use crate::strategies::{StrategyError, StrategyService, Target};
use crate::text::{self, NOTE_MAX_LEN};
use crate::users::username_norm;

/// Colour of a tag a preset creates: the client's `Colors.grey` (ARGB), its neutral choice.
pub const PRESET_TAG_COLOR: i64 = 0xFF9E_9E9E;

/// The presets of one sysinfo upload. Empty strings count as absent (the client only sends
/// non-empty values).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presets {
    pub user_name: Option<String>,
    pub device_group_name: Option<String>,
    pub strategy_name: Option<String>,
    pub note: Option<String>,
    pub address_book: Option<AddressBookPreset>,
}

/// `preset-address-book-*`; only meaningful with a name.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AddressBookPreset {
    pub name: String,
    pub tag: Option<String>,
    pub alias: Option<String>,
    /// Stored encrypted, like any shared-book password.
    pub password: Option<Zeroizing<String>>,
    pub note: Option<String>,
}

impl std::fmt::Debug for AddressBookPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddressBookPreset")
            .field("name", &self.name)
            .field("tag", &self.tag)
            .field("alias", &self.alias)
            .field("has_password", &self.password.is_some())
            .finish_non_exhaustive()
    }
}

/// What the sysinfo transaction writes into the device row (`None`: leave as is).
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct RowPresets {
    pub owner_user_id: Option<i64>,
    pub device_group_id: Option<i64>,
    pub note: Option<String>,
}

impl RowPresets {
    /// Owner or device group change, which can change the effective strategy.
    pub fn moves_device(&self) -> bool {
        self.owner_user_id.is_some() || self.device_group_id.is_some()
    }
}

/// The row presets that apply to `row`, read in `tx`. The user and group found stay locked
/// until `tx` ends, so they cannot vanish before the row points at them.
pub(super) async fn resolve_row(
    tx: &impl ConnectionTrait,
    presets: &Presets,
    row: &device::Model,
) -> Result<RowPresets, DbErr> {
    let owner_user_id = match (&presets.user_name, row.owner_user_id) {
        (Some(name), None) => find_user(tx, name).await?,
        _ => None,
    };
    let device_group_id = match (&presets.device_group_name, row.device_group_id) {
        (Some(name), None) => find_device_group(tx, name).await?,
        _ => None,
    };
    let note_unset = row.note.as_deref().is_none_or(str::is_empty);
    let note = presets
        .note
        .clone()
        .filter(|note| note_unset && valid_note(note));
    log_unresolved(row, presets, owner_user_id, device_group_id);
    Ok(RowPresets {
        owner_user_id,
        device_group_id,
        note,
    })
}

async fn find_user(tx: &impl ConnectionTrait, name: &str) -> Result<Option<i64>, DbErr> {
    let Some(norm) = username_norm(name.trim()) else {
        return Ok(None);
    };
    let found = user::Entity::find()
        .filter(user::Column::UsernameNorm.eq(norm))
        .lock_shared()
        .one(tx)
        .await?;
    Ok(found.map(|u| u.id))
}

async fn find_device_group(tx: &impl ConnectionTrait, name: &str) -> Result<Option<i64>, DbErr> {
    let found = device_group::Entity::find()
        .filter(device_group::Column::Name.eq(name.trim()))
        .lock_shared()
        .one(tx)
        .await?;
    Ok(found.map(|g| g.id))
}

fn valid_note(note: &str) -> bool {
    text::problem(note, NOTE_MAX_LEN, true).is_none()
}

fn log_unresolved(row: &device::Model, p: &Presets, owner: Option<i64>, group: Option<i64>) {
    let id = row.rustdesk_id.as_str();
    if p.user_name.is_some() && row.owner_user_id.is_none() && owner.is_none() {
        tracing::info!(rustdesk_id = id, "preset-user-name names no user; ignored");
    }
    if p.device_group_name.is_some() && row.device_group_id.is_none() && group.is_none() {
        tracing::info!(
            rustdesk_id = id,
            "preset-device-group-name names no group; ignored"
        );
    }
}

/// `preset-strategy-name`, after the sysinfo commit.
pub(super) async fn apply_strategy(
    strategies: &StrategyService,
    device: &device::Model,
    name: &str,
) {
    let id = device.rustdesk_id.as_str();
    let strategy = match strategies.get_by_name(name).await {
        Ok(strategy) => strategy,
        Err(StrategyError::NotFound) => {
            tracing::info!(
                rustdesk_id = id,
                "preset-strategy-name names no strategy; ignored"
            );
            return;
        }
        Err(err) => return log_failure(id, "preset-strategy-name", &err),
    };
    let target = Target::Device(device.id);
    match strategies.assign_if_unassigned(target, strategy.id).await {
        Ok(_) => {}
        Err(StrategyError::TargetNotFound(_) | StrategyError::NotFound) => {}
        Err(err) => log_failure(id, "preset-strategy-name", &err),
    }
}

/// `preset-address-book-*`, after the sysinfo commit.
pub(super) async fn apply_address_book(
    books: &AddressBookService,
    device: &device::Model,
    preset: &AddressBookPreset,
) {
    let id = device.rustdesk_id.as_str();
    let book = match books.find_shared_by_name(&preset.name).await {
        Ok(book) => book,
        Err(AddressBookError::NotFound | AddressBookError::AmbiguousName) => {
            tracing::info!(
                rustdesk_id = id,
                "preset-address-book-name names no single shared address book; ignored"
            );
            return;
        }
        Err(err) => return log_failure(id, "preset-address-book-name", &err),
    };
    if let Some(tag) = &preset.tag {
        let tag = Tag {
            name: tag.clone(),
            color: PRESET_TAG_COLOR,
        };
        if let Err(err) = books.add_tag(Actor::Admin, &book.guid, tag).await {
            return log_failure(id, "preset-address-book-tag", &err);
        }
    }
    let patch = match PeerPatch::from_json(peer_json(device, preset)) {
        Ok(patch) => patch,
        Err(err) => return log_failure(id, "preset-address-book-*", &err),
    };
    match books.add_peer(Actor::Admin, &book.guid, patch).await {
        Ok(()) | Err(AddressBookError::PeerExists) => {}
        Err(err) => log_failure(id, "preset-address-book-name", &err),
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
    if let Some(tag) = &preset.tag {
        peer.insert("tags".to_owned(), json!([tag]));
    }
    Value::Object(peer)
}

fn log_failure(rustdesk_id: &str, preset: &str, err: &dyn std::error::Error) {
    tracing::warn!(rustdesk_id, preset, error = %err, "device preset not applied");
}
