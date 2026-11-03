//! [`DeviceService::heartbeat`]: the hot path (02 §4.1, 03 §2.1), every 3–15 s per device.
//!
//! **Steady path, no database access**: a known device (identity cached in [`Presence`]) whose
//! client already holds the current strategy `modified_at` and has a sysinfo costs a few map
//! lookups: presence update, disconnect queue, strategy version, pending confirmation. The
//! `last_seen_at` it refreshes is written in batches by the flush task
//! ([`LastSeenFlusher`](crate::devices::LastSeenFlusher)).
//!
//! **Strategy** (05 §4.3): sent when the client's `modified_at` differs from the server's
//! (`!=`, never `<`: the server value can go back after a restart on a slower clock). The
//! options go out in full, empty values included (the client deletes those), plus `""` for
//! every key sent earlier that is no longer there (`strategy_keys`). Reading the device row
//! and the stored keys happens only then. The device's owner and group come from its row, so
//! anything that moves a device must invalidate the strategy cache (as the registry, the
//! console and deploy / assign do); `DeviceInputs::changed_at` is not consulted here.
//!
//! A failure while preparing the strategy is logged and the reply goes out without it (the
//! client keeps its `modified_at` and asks again next time); disconnects already taken are
//! not lost that way.

use std::net::IpAddr;

use deskhaven_db::entity::device::Entity as Devices;
use sea_orm::{DbErr, EntityTrait};

use super::{DeviceService, Inner};
use crate::devices::DeviceError;
use crate::devices::limits::HEARTBEAT_MIN_INTERVAL;
use crate::devices::strategy_keys::KeysError;
use crate::strategies::{DeviceInputs, Options, StrategyError};

/// A `POST /api/heartbeat` body, parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heartbeat {
    pub rustdesk_id: String,
    /// As sent: base64 of the device uuid.
    pub uuid: String,
    /// Source address (the resolved client IP).
    pub ip: IpAddr,
    /// Live connection ids (`conns`; absent = none).
    pub conns: Vec<i32>,
    /// The strategy version the client holds (`modified_at`; 0 before the first).
    pub modified_at: i64,
}

/// Result of [`DeviceService::heartbeat`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeartbeatOutcome {
    Reply(HeartbeatReply),
    /// Unknown id, or a uuid that does not match: nothing changed.
    NotFound,
    /// Within [`HEARTBEAT_MIN_INTERVAL`] of the device's previous heartbeat: nothing changed.
    TooFrequent,
}

/// What the reply carries; every part is optional on the wire.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeartbeatReply {
    /// The server holds no sysinfo of the device: ask for one (`sysinfo` key).
    pub sysinfo: bool,
    /// Connections to close (`disconnect`), all among the reported `conns`.
    pub disconnect: Vec<i32>,
    /// The strategy to apply, when the client's version differs.
    pub strategy: Option<StrategyPush>,
}

/// `strategy.config_options` and `modified_at` of a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategyPush {
    /// Empty values included: each deletes that option on the client.
    pub config_options: Options,
    /// What the client stores and reports back.
    pub modified_at: i64,
}

#[derive(Debug, thiserror::Error)]
enum PushError {
    #[error(transparent)]
    Keys(#[from] KeysError),
    #[error(transparent)]
    Strategy(#[from] StrategyError),
    #[error("device storage failed")]
    Database(#[from] DbErr),
}

impl DeviceService {
    /// Serves one heartbeat (see the module documentation).
    pub async fn heartbeat(&self, beat: Heartbeat) -> Result<HeartbeatOutcome, DeviceError> {
        let Some(identity) = self.identify(&beat.rustdesk_id, &beat.uuid).await? else {
            return Ok(HeartbeatOutcome::NotFound);
        };
        let inner = &self.inner;
        let (id, conns) = (&beat.rustdesk_id, &beat.conns);
        if !inner
            .presence
            .try_heartbeat(id, beat.ip, conns, HEARTBEAT_MIN_INTERVAL)
        {
            return Ok(HeartbeatOutcome::TooFrequent);
        }
        let disconnect = inner.disconnects.take(&beat.rustdesk_id, &beat.conns);
        let strategy = inner
            .strategy_for(identity.device_id, beat.modified_at)
            .await;
        Ok(HeartbeatOutcome::Reply(HeartbeatReply {
            sysinfo: !identity.has_sysinfo,
            disconnect,
            strategy,
        }))
    }
}

impl Inner {
    /// The strategy to send device `device_id` whose client holds `client_version`, if any.
    async fn strategy_for(&self, device_id: i64, client_version: i64) -> Option<StrategyPush> {
        let inputs = DeviceInputs {
            device_id,
            ..DeviceInputs::default()
        };
        let current = self.strategies.modified_at(&inputs);
        let result = match client_version == current {
            true => self.confirm(device_id, current).await,
            false => self.push(device_id).await,
        };
        result.unwrap_or_else(|err| {
            tracing::warn!(device_id, error = %err, "heartbeat strategy skipped");
            None
        })
    }

    /// Confirmation never sends anything.
    async fn confirm(
        &self,
        device_id: i64,
        version: i64,
    ) -> Result<Option<StrategyPush>, PushError> {
        self.deliveries.confirm(device_id, version).await?;
        Ok(None)
    }

    async fn push(&self, device_id: i64) -> Result<Option<StrategyPush>, PushError> {
        let Some(row) = Devices::find_by_id(device_id).one(&self.db).await? else {
            return Ok(None);
        };
        let effective = self.strategies.effective(&DeviceInputs::from(&row)).await?;
        let config_options = self.deliveries.prepare(device_id, &effective).await?;
        Ok(Some(StrategyPush {
            config_options,
            modified_at: effective.modified_at,
        }))
    }
}
