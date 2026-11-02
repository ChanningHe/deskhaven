//! The signal server's peer store over the device registry (07-hbbs-fork §3.1): what
//! hbbs registers lands in `devices`, the single source of truth for registered devices.
//! `serve` starts the signal server on it (`signal_server`, M1-06).

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use deskhaven_core::devices::{DeviceError, DeviceRepository, Registration};
use deskhaven_db::time::{self, TimestampMs};
use deskhaven_signal::{PeerRecord, PeerStore, PeerUpsert, StoreError};

/// [`PeerStore`] backed by [`DeviceRepository`].
pub struct SignalPeerStore {
    devices: DeviceRepository,
}

impl SignalPeerStore {
    pub fn new(devices: DeviceRepository) -> Self {
        Self { devices }
    }
}

#[async_trait]
impl PeerStore for SignalPeerStore {
    async fn load_page(
        &self,
        after: Option<&str>,
        limit: usize,
    ) -> Result<Vec<PeerRecord>, StoreError> {
        let limit = u64::try_from(limit).map_err(StoreError::backend)?;
        let page = self
            .devices
            .registrations_page(after, limit)
            .await
            .map_err(StoreError::backend)?;
        Ok(page
            .into_iter()
            .map(|d| record(d.rustdesk_id, d.uuid, d.pk, &d.register_ip))
            .collect())
    }

    async fn get(&self, id: &str) -> Result<Option<PeerRecord>, StoreError> {
        let device = self
            .devices
            .find_by_rustdesk_id(id)
            .await
            .map_err(StoreError::backend)?;
        Ok(device.map(|d| record(d.rustdesk_id, d.uuid, d.pk, &d.register_ip)))
    }

    async fn upsert_pk(&self, rec: &PeerUpsert) -> Result<(), StoreError> {
        let reg = Registration {
            rustdesk_id: rec.id.clone(),
            uuid: rec.uuid.clone(),
            pk: rec.pk.clone(),
            ip: rec.ip,
            at: timestamp(rec.registered_at)?,
        };
        self.devices
            .upsert_registration(&reg)
            .await
            .map_err(|err| match err {
                DeviceError::UuidMismatch { rustdesk_id } => {
                    StoreError::UuidMismatch { id: rustdesk_id }
                }
                other => StoreError::backend(other),
            })
    }
}

fn record(id: String, uuid: Vec<u8>, pk: Vec<u8>, register_ip: &str) -> PeerRecord {
    PeerRecord {
        id,
        uuid,
        pk,
        // Always written from an `IpAddr`; anything else reads as unknown.
        ip: register_ip.parse().ok(),
    }
}

fn timestamp(at: SystemTime) -> Result<TimestampMs, StoreError> {
    at.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .and_then(time::from_millis)
        .ok_or_else(|| StoreError::backend("registration time out of range"))
}

#[cfg(test)]
mod tests;
