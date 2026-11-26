//! Presets are self-reported: whoever registers an id with a uuid of their choosing can send
//! any of them. They only apply when an administrator opted in, and never while
//! `devices.verify_uuid` is off (anyone knowing an id could send the first sysinfo of a migrated
//! device).
//!
//! Only the first sysinfo a row ever stores applies them, decided under the row lock, so an
//! administrator's later change is never undone by the device. Each preset only fills what is
//! unset; failures are logged and skipped, never failing the upload.

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

/// Empty strings count as absent (the client only sends non-empty values).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Presets {
    pub user_name: Option<String>,
    pub device_group_name: Option<String>,
    pub strategy_name: Option<String>,
    pub note: Option<String>,
    pub address_book: Option<AddressBookPreset>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct AddressBookPreset {
    pub name: String,
    pub tag: Option<String>,
    pub alias: Option<String>,
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

pub(super) fn scope(settings: &Snapshot) -> AcceptPresets {
    match settings.get(&DEVICES_VERIFY_UUID) {
        true => settings.get(&DEVICES_ACCEPT_PRESETS),
        false => AcceptPresets::Off,
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct RowPresets {
    pub owner_user_id: Option<i64>,
    pub device_group_id: Option<i64>,
    pub note: Option<String>,
}

impl RowPresets {
    pub fn moves_device(&self) -> bool {
        self.owner_user_id.is_some() || self.device_group_id.is_some()
    }
}

/// The user and group found stay key-share locked until `tx` ends, so they cannot vanish before
/// the row points at them.
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
