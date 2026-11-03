//! Deployment presets: the `preset-*` fields a client adds to its sysinfo (03 §2.2;
//! `src/hbbs_http/sync.rs:146-188`), set by the custom client's built-in options or its
//! config, to place a freshly installed device without an administrator.
//!
//! Presets are **self-reported**: whoever registers an id with a uuid of their choosing can
//! send any of them. They therefore only apply when an administrator opted in:
//!
//! - `devices.accept_presets` (default `off`): `row` applies owner, device group, note and
//!   strategy; `all` also the address book entry.
//! - Never while `devices.verify_uuid` is off: anyone knowing an id could then send the first
//!   sysinfo of a device that registered but never reported (e.g. every device migrated
//!   from an OSS hbbs).
//! - TODO(M5): with `devices.require_deploy`, accept them only from deployed devices.
//!
//! **When**: only with the first sysinfo a device row ever stores (`sysinfo_hash` was
//! `NULL`), decided under the row lock, so concurrent first uploads apply one set. Later
//! uploads never apply them again: an administrator's change (console, `--assign`) is never
//! undone by the device. Presets that could not be applied then (unknown name) stay
//! unapplied; place the device by `--assign` or in the console instead.
//!
//! **What**, each only where nothing is set yet (never replacing a value):
//!
//! | field | effect |
//! |---|---|
//! | `preset-user-name` | owner = the user with that username (case-insensitive) |
//! | `preset-device-group-name` | device group = the group with exactly that name |
//! | `preset-note` | device note |
//! | `preset-strategy-name` | device-level strategy assignment = the strategy with that name |
//! | `preset-address-book-*` | see [`preset_book`](super::preset_book): a peer entry written **as the device's owner** |
//!
//! Unknown or ambiguous names, invalid values, missing rights and full books are skipped
//! (logged); they never fail the sysinfo upload. Owner, device group and note are written in
//! the sysinfo transaction; the strategy and the address book entry right after it commits
//! (best effort: a crash in between loses them, like an unknown name would).

use deskhaven_config::settings::{
    AcceptPresets, DEVICES_ACCEPT_PRESETS, DEVICES_VERIFY_UUID, Snapshot,
};
use deskhaven_db::entity::{device, device_group, user};
use sea_orm::sea_query::LockType;
use sea_orm::{ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect};
use zeroize::Zeroizing;

use crate::strategies::{StrategyError, StrategyService, Target};
use crate::text::{self, NOTE_MAX_LEN};
use crate::users::username_norm;

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

/// Which presets the current settings let through (see the module documentation).
pub(super) fn scope(settings: &Snapshot) -> AcceptPresets {
    match settings.get(&DEVICES_VERIFY_UUID) {
        true => settings.get(&DEVICES_ACCEPT_PRESETS),
        false => AcceptPresets::Off,
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

/// The row presets that apply to `row`, read in `tx`. The user and group found stay
/// key-share locked until `tx` ends, so they cannot vanish before the row points at them
/// (PostgreSQL; SQLite's write transaction excludes other writers).
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

pub(super) async fn find_user(tx: &impl ConnectionTrait, name: &str) -> Result<Option<i64>, DbErr> {
    let Some(norm) = username_norm(name.trim()) else {
        return Ok(None);
    };
    let found = user::Entity::find()
        .filter(user::Column::UsernameNorm.eq(norm))
        .lock(LockType::KeyShare)
        .one(tx)
        .await?;
    Ok(found.map(|u| u.id))
}

pub(super) async fn find_device_group(
    tx: &impl ConnectionTrait,
    name: &str,
) -> Result<Option<i64>, DbErr> {
    let found = device_group::Entity::find()
        .filter(device_group::Column::Name.eq(name.trim()))
        .lock(LockType::KeyShare)
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

pub(super) fn log_failure(rustdesk_id: &str, preset: &str, err: &dyn std::error::Error) {
    tracing::warn!(rustdesk_id, preset, error = %err, "device preset not applied");
}
