//! What `--deploy` / `--assign` (M2-19) share: who asks, and the signal server's side of the
//! registry. The signal server keeps registrations in memory and writes them behind
//! (07 §3.1), so a change made here must (a) not contradict a registration it holds but has
//! not stored yet ([`LiveRegistrations`]), and (b) be re-read by it afterwards
//! ([`DeviceService::on_registry_changed`], like [`DeviceService::on_device_deleted`]).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use subtle::ConstantTimeEq;

use super::{DeletedHook, DeviceService, Inner};
use crate::auth::limiter::ClientAddr;
use crate::console_audit::Actor;

/// Who asks for a device change with a credential (an API token): the audit actor, and the
/// source the per-network limits and the audit row use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requester {
    pub actor: Actor,
    pub client: ClientAddr,
}

/// The signal server's in-memory registrations (`SignalHandle::live_uuid`).
pub trait LiveRegistrations: Send + Sync {
    /// The uuid the signal server holds for `rustdesk_id`, stored or not; `None` when it
    /// holds none or cannot tell (not running).
    fn uuid_of<'a>(
        &'a self,
        rustdesk_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<Vec<u8>>> + Send + 'a>>;
}

#[derive(Default)]
pub(super) struct SignalRegistry {
    changed: RwLock<Vec<DeletedHook>>,
    live: RwLock<Option<Arc<dyn LiveRegistrations>>>,
}

impl DeviceService {
    /// Runs `hook` with the id after every committed `--deploy` that wrote a row. `serve`
    /// uses it to make the signal server re-read the id (`SignalHandle::reload_peer`).
    pub fn on_registry_changed(&self, hook: DeletedHook) {
        let registry = &self.inner.registry;
        let mut hooks = registry
            .changed
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        hooks.push(hook);
    }

    /// Where `--deploy` looks up registrations the signal server has not stored yet. Without
    /// one (no signal server in this process), only the database counts.
    pub fn set_live_registrations(&self, live: Arc<dyn LiveRegistrations>) {
        let registry = &self.inner.registry;
        *registry
            .live
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(live);
    }
}

impl Inner {
    pub(super) fn registry_changed(&self, rustdesk_id: &str) {
        let hooks = self
            .registry
            .changed
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for hook in hooks {
            hook(rustdesk_id);
        }
    }

    pub(super) async fn live_uuid(&self, rustdesk_id: &str) -> Option<Vec<u8>> {
        let live = self
            .registry
            .live
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()?;
        live.uuid_of(rustdesk_id).await
    }
}

/// The token endpoints' uuid check: always, whatever `devices.verify_uuid` says (tokens
/// travel in scripts, they prove nothing about the machine), in constant time.
pub(super) fn same_uuid(stored: &[u8], given: &[u8]) -> bool {
    stored.ct_eq(given).into()
}
