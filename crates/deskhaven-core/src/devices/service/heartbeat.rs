//! Steady path, no database access: a known device whose client holds the current strategy costs
//! a few map lookups. The strategy is sent when the client's `modified_at` differs (`!=`, never
//! `<`: the server value can go back after a restart on a slower clock), with `""` for every key
//! sent earlier that is gone. Anything that moves a device must invalidate the strategy cache. A
//! failure preparing the strategy is logged and the reply goes out without it.

use std::time::Duration;

use deskhaven_db::entity::device::Entity as Devices;
use sea_orm::{DbErr, EntityTrait};

use super::{DeviceService, Inner};
use crate::auth::limiter::ClientAddr;
use crate::devices::DeviceError;
use crate::devices::limits::HEARTBEAT_MIN_INTERVAL;
use crate::devices::strategy_keys::KeysError;
use crate::strategies::{DeviceInputs, Options, StrategyError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heartbeat {
    pub rustdesk_id: String,
    pub uuid: String,
    pub client: ClientAddr,
    pub conns: Vec<i32>,
    pub modified_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeartbeatOutcome {
    Reply(HeartbeatReply),
    NotFound,
    TooFrequent,
    /// The request needed a lookup and the client network is over its quota.
    RateLimited {
        retry_after: Duration,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeartbeatReply {
    pub sysinfo: bool,
    pub disconnect: Vec<i32>,
    pub strategy: Option<StrategyPush>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategyPush {
    /// Empty values included: each deletes that option on the client.
    pub config_options: Options,
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
    /// Only a request that needs a lookup is charged to the client network's quota.
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
