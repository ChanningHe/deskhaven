//! Who holds a RustDesk id, for a login that binds a session to the device a client names
//! (the OIDC client login: nothing else proves the device there, see
//! `deskhaven_client_api::oidc`).
//!
//! The binding is what the signal server enforces: an id registered with one uuid answers
//! any other uuid's RegisterPk with `UUID_MISMATCH`, so a real client's id and uuid always
//! agree with it. Asked like `--deploy` asks: the stored row first (its uuid never changes),
//! else the signal server's memory, which holds registrations it has not stored yet.
//! Always compared, whatever `devices.verify_uuid` says (that setting is about the device
//! endpoints; the signal server never relaxes the binding).

use base64_022::Engine as _;

use super::registry::same_uuid;
use super::{DeviceService, LiveUnknown};
use crate::crypto::b64::KEY_B64;
use crate::devices::DeviceError;

/// Whom a RustDesk id belongs to, compared with the uuid a client sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdHolder {
    /// Registered with this uuid.
    ThisDevice,
    /// Registered with another uuid (or the uuid sent is not base64).
    OtherDevice,
    /// Neither stored nor held by the signal server: the id has not registered here yet.
    Nobody,
    /// Not stored, and the signal server did not answer in time ([`LiveUnknown`]).
    Unknown,
}

impl DeviceService {
    /// Whom `rustdesk_id` belongs to, for `uuid` as the client sends it (base64 of the raw
    /// uuid of its RegisterPk). Served from the identity cache when the device is in it.
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
