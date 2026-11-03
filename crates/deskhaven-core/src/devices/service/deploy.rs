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
//!   one the signal server registered. An unknown id is registered here, bound to this uuid
//!   and key, as the signal server would on its first RegisterPk. An id bound to another
//!   uuid is [`DeployOutcome::IdTaken`] and left alone.
//!
//! Each statement is atomic on both backends, so this needs no transaction or lock, exactly
//! like [`DeviceRepository::upsert_registration`](crate::devices::DeviceRepository): the
//! update is conditional on the uuid, the insert does nothing on a conflicting id.

use std::net::IpAddr;

use base64_022::Engine as _;
use deskhaven_config::settings::DEVICES_REQUIRE_DEPLOY;
use deskhaven_db::entity::device::{self, DeviceStatus, Entity as Devices};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, TryInsertResult};

use super::DeviceService;
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
    /// The caller's address, recorded as the registration address of a new id.
    pub ip: IpAddr,
}

/// The `result` of a deploy (the client matches these four).
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
}

/// A validated request.
struct Keys {
    rustdesk_id: String,
    uuid: Vec<u8>,
    pk: Vec<u8>,
    ip: String,
}

impl DeviceService {
    /// Deploys a device (see the module documentation).
    pub async fn deploy(&self, req: DeployRequest) -> Result<DeployOutcome, DeviceError> {
        if !self.inner.settings.snapshot().get(&DEVICES_REQUIRE_DEPLOY) {
            return Ok(DeployOutcome::NotEnabled);
        }
        let Some(keys) = parse(req) else {
            return Ok(DeployOutcome::InvalidInput);
        };
        let db = &self.inner.db;
        let now = time::now();
        let deployed = mark(db, &keys, now).await?
            || insert(db, &keys, now).await?
            // Lost an insert race: fine if the winner registered the same uuid.
            || mark(db, &keys, now).await?;
        Ok(match deployed {
            true => DeployOutcome::Deployed,
            false => DeployOutcome::IdTaken,
        })
    }
}

fn parse(req: DeployRequest) -> Option<Keys> {
    if !is_deployable_id(&req.rustdesk_id) {
        return None;
    }
    let uuid = KEY_B64.decode(&req.uuid).ok()?;
    let pk = KEY_B64.decode(&req.pk).ok()?;
    let usable = !uuid.is_empty() && uuid.len() <= UUID_MAX_BYTES && pk.len() == PK_BYTES;
    usable.then(|| Keys {
        rustdesk_id: req.rustdesk_id,
        uuid,
        pk,
        ip: req.ip.to_string(),
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

/// `true` if the id is registered with this uuid (now marked deployed).
async fn mark(db: &DatabaseConnection, keys: &Keys, now: TimestampMs) -> Result<bool, DbErr> {
    let updated = Devices::update_many()
        .col_expr(device::Column::DeployedAt, Expr::value(now))
        .col_expr(device::Column::UpdatedAt, Expr::value(now))
        .filter(device::Column::RustdeskId.eq(&keys.rustdesk_id))
        .filter(device::Column::Uuid.eq(keys.uuid.clone()))
        .exec(db)
        .await?;
    Ok(updated.rows_affected > 0)
}

/// `true` if the id was unknown and is now registered, deployed.
async fn insert(db: &DatabaseConnection, keys: &Keys, now: TimestampMs) -> Result<bool, DbErr> {
    let row = device::ActiveModel {
        id: NotSet,
        rustdesk_id: Set(keys.rustdesk_id.clone()),
        uuid: Set(keys.uuid.clone()),
        pk: Set(keys.pk.clone()),
        status: Set(DeviceStatus::Active),
        deployed_at: Set(Some(now)),
        first_seen_at: Set(now),
        last_seen_at: Set(now),
        last_ip: Set(keys.ip.clone()),
        register_ip: Set(keys.ip.clone()),
        register_at: Set(now),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    };
    let res = Devices::insert(row)
        .on_conflict_do_nothing_on([device::Column::RustdeskId])
        .exec_without_returning(db)
        .await?;
    Ok(matches!(res, TryInsertResult::Inserted(n) if n > 0))
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
