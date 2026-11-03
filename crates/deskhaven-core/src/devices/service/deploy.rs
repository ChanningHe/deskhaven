//! [`DeviceService::deploy`]: `rustdesk --deploy` (03 §2.8, `src/ui_interface.rs:1133-1200`),
//! called with an API token holding the `deploy` scope.
//!
//! - Only while `devices.require_deploy` is on; otherwise [`DeployOutcome::NotEnabled`] and
//!   nothing is written (the client: "Server does not require deployment.").
//! - The request names the id to deploy (the device's own, or a new one it will take on
//!   success), its uuid and its public key (both base64). An id the signal server could not
//!   register, or a uuid / key that is not base64 of a usable value, is
//!   [`DeployOutcome::InvalidInput`].
//! - An id registered with this uuid is marked deployed (`deployed_at`); its key stays the
//!   one the signal server registered. An id bound to another uuid, in the database or in
//!   the signal server's memory (a registration not stored yet, [`LiveRegistrations`]), is
//!   [`DeployOutcome::IdTaken`] and left alone (logged). The uuid is always compared, whatever
//!   `devices.verify_uuid` says.
//! - An unknown id is registered here, bound to this uuid and key, as the signal server
//!   would on its first RegisterPk, within the new-id quota (`new_ids`;
//!   [`DeployOutcome::TooMany`] beyond it). If the signal server does not say in time
//!   whether it holds the id, [`DeployOutcome::Unavailable`] (retry); a stopped one holds
//!   nothing.
//! - An id already deployed with this uuid is answered OK at once: no lookup, no write, no
//!   audit row, and the first deploy time is kept.
//!
//! The write and its console audit row (as the requester) share one write transaction, run
//! in its own task; after the commit of a new id the registry-changed hooks make the signal
//! server load it.
//!
//! [`LiveRegistrations`]: super::LiveRegistrations

use std::sync::Arc;
use std::time::Duration;

use base64_022::Engine as _;
use deskhaven_config::settings::DEVICES_REQUIRE_DEPLOY;
use deskhaven_db::begin_write;
use deskhaven_db::entity::device::{self, DeviceStatus, Entity as Devices};
use deskhaven_db::time::TimestampMs;
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter, QuerySelect, TryInsertResult,
};
use serde_json::json;

use super::registry::same_uuid;
use super::{DeviceService, Inner, LiveUnknown, Requester};
use crate::console_audit::{self, NewConsoleAudit};
use crate::crypto::b64::KEY_B64;
use crate::devices::DeviceError;

/// Ids the signal server registers: this many bytes at least (`register_pk`)...
pub const DEPLOY_ID_MIN_LEN: usize = 6;
/// ...and at most (`MAX_PEER_ID_LEN`).
pub const DEPLOY_ID_MAX_LEN: usize = 253;
/// Longest uuid the signal server registers, in bytes (`MAX_REGISTER_UUID_LEN`).
const UUID_MAX_BYTES: usize = 128;
/// An ed25519 public key.
const PK_BYTES: usize = 32;

/// A `POST /api/devices/deploy` body, as sent (validated by [`DeviceService::deploy`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeployRequest {
    pub rustdesk_id: String,
    /// Base64 of the device uuid.
    pub uuid: String,
    /// Base64 of the device public key.
    pub pk: String,
    /// Who asks; its address is recorded as the registration address of a new id.
    pub by: Requester,
}

/// The `result` of a deploy (the client matches the first four).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeployOutcome {
    /// `OK`.
    Deployed,
    /// `NOT_ENABLED`: `devices.require_deploy` is off.
    NotEnabled,
    /// `INVALID_INPUT`.
    InvalidInput,
    /// `ID_TAKEN`: the id belongs to another uuid.
    IdTaken,
    /// The new-id quota of the requester's network or actor is used up (HTTP 429).
    TooMany { retry_after: Duration },
    /// The signal server did not say in time whether it holds the id (HTTP 503); nothing
    /// was written.
    Unavailable { retry_after: Duration },
}

/// `Retry-After` of [`DeployOutcome::Unavailable`].
pub const LIVE_RETRY_AFTER: Duration = Duration::from_secs(5);

/// A validated request.
struct Keys {
    rustdesk_id: String,
    uuid: Vec<u8>,
    pk: Vec<u8>,
}

impl DeviceService {
    /// Deploys a device on behalf of `req.by` (see the module documentation).
    pub async fn deploy(&self, req: DeployRequest) -> Result<DeployOutcome, DeviceError> {
        if !self.inner.settings.snapshot().get(&DEVICES_REQUIRE_DEPLOY) {
            return Ok(DeployOutcome::NotEnabled);
        }
        let by = req.by;
        let Some(keys) = parse(req) else {
            return Ok(DeployOutcome::InvalidInput);
        };
        let existing = self.inner.find(&keys.rustdesk_id).await?;
        let live = match &existing {
            Some(row) if same_uuid(&row.uuid, &keys.uuid) && row.deployed_at.is_some() => {
                return Ok(DeployOutcome::Deployed);
            }
            Some(_) => None,
            // Asked before the write lock is taken: a round trip to the signal server.
            None => match self.inner.live_uuid(&keys.rustdesk_id).await {
                Ok(live) => live,
                Err(LiveUnknown) => {
                    let retry_after = LIVE_RETRY_AFTER;
                    return Ok(DeployOutcome::Unavailable { retry_after });
                }
            },
        };
        self.detached(move |inner| inner.deploy_keys(keys, live, by))
            .await
    }
}

impl Inner {
    async fn deploy_keys(
        self: Arc<Self>,
        keys: Keys,
        live: Option<Vec<u8>>,
        by: Requester,
    ) -> Result<DeployOutcome, DeviceError> {
        let now = self.now()?;
        let tx = begin_write(&self.db).await?;
        let registered = match find_locked(&tx, &keys.rustdesk_id).await? {
            Some(row) => same_uuid(&row.uuid, &keys.uuid).then_some(false),
            None if live
                .as_deref()
                .is_some_and(|uuid| !same_uuid(uuid, &keys.uuid)) =>
            {
                None
            }
            None => {
                if let Err(retry_after) = self.new_ids.try_charge(by.client, by.actor) {
                    return Ok(DeployOutcome::TooMany { retry_after });
                }
                insert_or_match(&tx, &keys, &by, now).await?
            }
        };
        let Some(registered) = registered else {
            let id = keys.rustdesk_id.as_str();
            tracing::warn!(rustdesk_id = id, actor = ?by.actor, ip = %by.client.addr, "deploy refused: the id belongs to another device");
            return Ok(DeployOutcome::IdTaken);
        };
        mark(&tx, &keys.rustdesk_id, now).await?;
        let entry = audit_entry(&keys.rustdesk_id, registered, by);
        console_audit::insert(&tx, &entry, now.timestamp_millis()).await?;
        tx.commit().await?;
        // Only a new row is news to the signal server (it never reads `deployed_at`), and a
        // reload of an id it holds could undo a newer key it has not stored yet.
        if registered {
            self.registry_changed(&keys.rustdesk_id);
        }
        Ok(DeployOutcome::Deployed)
    }
}

fn parse(req: DeployRequest) -> Option<Keys> {
    if !is_deployable_id(&req.rustdesk_id) {
        return None;
    }
    let uuid = KEY_B64.decode(&req.uuid).ok()?;
    let pk = KEY_B64.decode(&req.pk).ok()?;
    let usable = !uuid.is_empty() && uuid.len() <= UUID_MAX_BYTES && pk.len() == PK_BYTES;
    usable.then_some(Keys {
        rustdesk_id: req.rustdesk_id,
        uuid,
        pk,
    })
}

/// An id the signal server registers and the client accepts as a custom id: ASCII letters,
/// digits, `-` and `_`, within the signal server's length bounds.
fn is_deployable_id(id: &str) -> bool {
    (DEPLOY_ID_MIN_LEN..=DEPLOY_ID_MAX_LEN).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

async fn find_locked(tx: &DatabaseTransaction, id: &str) -> Result<Option<device::Model>, DbErr> {
    Devices::find()
        .filter(device::Column::RustdeskId.eq(id))
        .lock_exclusive()
        .one(tx)
        .await
}

/// Registers the unknown id: `Some(true)`. A concurrent registration of the same id
/// (PostgreSQL) makes the insert do nothing; then `Some(false)` if the winner has this uuid,
/// else `None` (taken).
async fn insert_or_match(
    tx: &DatabaseTransaction,
    keys: &Keys,
    by: &Requester,
    now: TimestampMs,
) -> Result<Option<bool>, DbErr> {
    if insert(tx, keys, by, now).await? {
        return Ok(Some(true));
    }
    let winner = find_locked(tx, &keys.rustdesk_id).await?;
    Ok(winner
        .filter(|row| same_uuid(&row.uuid, &keys.uuid))
        .map(|_| false))
}

/// Sets `deployed_at` of the (locked) row, unless set: the first deploy time is kept.
async fn mark(tx: &DatabaseTransaction, id: &str, now: TimestampMs) -> Result<(), DbErr> {
    Devices::update_many()
        .col_expr(device::Column::DeployedAt, Expr::value(now))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::RustdeskId.eq(id))
        .filter(device::Column::DeployedAt.is_null())
        .exec(tx)
        .await
        .map(drop)
}

/// `true` if the id was unknown and is now registered.
async fn insert(
    tx: &DatabaseTransaction,
    keys: &Keys,
    by: &Requester,
    now: TimestampMs,
) -> Result<bool, DbErr> {
    let ip = by.client.addr.to_string();
    let row = device::ActiveModel {
        id: NotSet,
        rustdesk_id: Set(keys.rustdesk_id.clone()),
        uuid: Set(keys.uuid.clone()),
        pk: Set(keys.pk.clone()),
        status: Set(DeviceStatus::Active),
        first_seen_at: Set(now),
        last_seen_at: Set(now),
        last_ip: Set(ip.clone()),
        register_ip: Set(ip),
        register_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    let res = Devices::insert(row)
        .on_conflict_do_nothing_on([device::Column::RustdeskId])
        .exec_without_returning(tx)
        .await?;
    Ok(matches!(res, TryInsertResult::Inserted(n) if n > 0))
}

/// `registered`: the deploy created the row (else it marked an existing one).
fn audit_entry(rustdesk_id: &str, registered: bool, by: Requester) -> NewConsoleAudit {
    NewConsoleAudit {
        actor: by.actor,
        action: "device.deploy".to_owned(),
        target_type: "device".to_owned(),
        target_id: rustdesk_id.to_owned(),
        before: None,
        after: Some(json!({ "deployed": true, "registered": registered })),
        ip: Some(by.client.addr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployable_ids_are_what_the_signal_server_registers() {
        for ok in ["123456", "alice-pc_01", &"9".repeat(DEPLOY_ID_MAX_LEN)] {
            assert!(is_deployable_id(ok), "{ok:?}");
        }
        for bad in [
            "",
            "12345",
            "a b c d",
            "abc.def",
            "äbcdef",
            &"9".repeat(254),
        ] {
            assert!(!is_deployable_id(bad), "{bad:?}");
        }
    }
}
