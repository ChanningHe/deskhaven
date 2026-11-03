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

use std::time::Duration;

use deskhaven_db::entity::device::Entity as Devices;
use sea_orm::{DbErr, EntityTrait};

use super::{DeviceService, Inner};
use crate::auth::limiter::ClientAddr;
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
    /// The resolved client address (presence IP, per-network quota).
    pub client: ClientAddr,
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
    /// The request needed a lookup (unknown or not cached identity, uuid mismatch) and the
    /// client network is over [`HEARTBEAT_PER_IP_PER_MINUTE`](crate::devices::HEARTBEAT_PER_IP_PER_MINUTE).
    RateLimited {
        retry_after: Duration,
    },
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
    /// Only a request that needs a lookup (identity not cached, unknown id, uuid mismatch)
    /// is charged to the client network's quota: a known device costs no database access and
    /// is limited per device instead.
    pub async fn heartbeat(&self, beat: Heartbeat) -> Result<HeartbeatOutcome, DeviceError> {
        let inner = &self.inner;
        let (id, conns) = (&beat.rustdesk_id, &beat.conns);
        let cached = inner.presence.identity(id);
        let was_cached = cached.is_some();
        let known = match cached {
            Some(identity) => Some(identity),
            None => match self.admit_heartbeat(beat.client) {
                Ok(()) => self.load_identity(id).await?,
                Err(retry_after) => return Ok(HeartbeatOutcome::RateLimited { retry_after }),
            },
        };
        let verified = known.filter(|identity| inner.uuid_matches(&identity.uuid, &beat.uuid));
        let Some(identity) = verified else {
            // A mismatch against a cached identity is charged too (once): it is a probe.
            if was_cached && let Err(retry_after) = self.admit_heartbeat(beat.client) {
                return Ok(HeartbeatOutcome::RateLimited { retry_after });
            }
            return Ok(HeartbeatOutcome::NotFound);
        };
        if !inner
            .presence
            .try_heartbeat(id, beat.client.addr, conns, HEARTBEAT_MIN_INTERVAL)
        {
            return Ok(HeartbeatOutcome::TooFrequent);
        }
        let disconnect = inner.disconnects.take(&beat.rustdesk_id, &beat.conns);
        let strategy = inner
            .strategy_for(id, identity.device_id, beat.modified_at)
            .await;
        Ok(HeartbeatOutcome::Reply(HeartbeatReply {
            sysinfo: !identity.has_sysinfo,
            disconnect,
            strategy,
        }))
    }
}

impl Inner {
    /// The strategy to send device `rustdesk_id` (row `device_id`) whose client holds
    /// `client_version`, if any.
    async fn strategy_for(
        &self,
        rustdesk_id: &str,
        device_id: i64,
        client_version: i64,
    ) -> Option<StrategyPush> {
        let inputs = DeviceInputs {
            device_id,
            ..DeviceInputs::default()
        };
        let current = self.strategies.modified_at(&inputs);
        let result = match client_version == current {
            true => self.confirm(rustdesk_id, current).await,
            false => self.push(rustdesk_id, device_id).await,
        };
        result.unwrap_or_else(|err| {
            tracing::warn!(rustdesk_id, error = %err, "heartbeat strategy skipped");
            None
        })
    }

    /// Confirmation never sends anything.
    async fn confirm(
        &self,
        rustdesk_id: &str,
        version: i64,
    ) -> Result<Option<StrategyPush>, PushError> {
        self.deliveries.confirm(rustdesk_id, version).await?;
        Ok(None)
    }

    async fn push(
        &self,
        rustdesk_id: &str,
        device_id: i64,
    ) -> Result<Option<StrategyPush>, PushError> {
        let Some(row) = Devices::find_by_id(device_id).one(&self.db).await? else {
            return Ok(None);
        };
        let effective = self.strategies.effective(&DeviceInputs::from(&row)).await?;
        let config_options = self.deliveries.prepare(rustdesk_id, &effective).await?;
        Ok(Some(StrategyPush {
            config_options,
            modified_at: effective.modified_at,
        }))
    }
}
