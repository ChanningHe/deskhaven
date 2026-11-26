//! The binding is what the signal server enforces: an id registered with one uuid refuses any
//! other uuid, so a real client's id and uuid always agree with it. Always compared, whatever
//! `devices.verify_uuid` says.

use base64_022::Engine as _;

use super::registry::same_uuid;
use super::{DeviceService, LiveUnknown};
use crate::crypto::b64::KEY_B64;
use crate::devices::DeviceError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdHolder {
    ThisDevice,
    OtherDevice,
    Nobody,
    Unknown,
}

impl DeviceService {
    pub async fn id_holder(&self, rustdesk_id: &str, uuid: &str) -> Result<IdHolder, DeviceError> {
        let stored = match self.inner.presence.identity(rustdesk_id) {
            Some(identity) => Some(identity.uuid),
            None => self
                .load_identity(rustdesk_id)
                .await?
                .map(|identity| identity.uuid),
        };
        let held = match stored {
            Some(uuid) => uuid,
            None => match self.inner.live_uuid(rustdesk_id).await {
                Ok(Some(uuid)) => uuid,
                Ok(None) => return Ok(IdHolder::Nobody),
                Err(LiveUnknown) => return Ok(IdHolder::Unknown),
            },
        };
        let matches = KEY_B64
            .decode(uuid)
            .is_ok_and(|given| same_uuid(&held, &given));
        Ok(match matches {
            true => IdHolder::ThisDevice,
            false => IdHolder::OtherDevice,
        })
    }
}
