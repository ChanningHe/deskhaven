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

/// The signal server is running but did not say in time whether it holds an id: no answer
/// is no green light (a registration not stored yet could be lost to the deploy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveUnknown;

/// A pending [`LiveRegistrations::uuid_of`].
pub type LiveLookup<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, LiveUnknown>> + Send + 'a>>;

/// The signal server's in-memory registrations (`SignalHandle::live_uuid`).
pub trait LiveRegistrations: Send + Sync {
    /// The uuid the signal server holds for `rustdesk_id`, stored or not; `Ok(None)` when it
    /// holds none or is not running (it loads the database when it starts).
    fn uuid_of<'a>(&'a self, rustdesk_id: &'a str) -> LiveLookup<'a>;
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

    /// `Ok(None)` without a signal server in this process.
    pub(super) async fn live_uuid(
        &self,
        rustdesk_id: &str,
    ) -> Result<Option<Vec<u8>>, LiveUnknown> {
        let live = self
            .registry
            .live
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match live {
            Some(live) => live.uuid_of(rustdesk_id).await,
            None => Ok(None),
        }
    }
}

/// The token endpoints' uuid check: always, whatever `devices.verify_uuid` says (tokens
/// travel in scripts, they prove nothing about the machine), in constant time.
pub(super) fn same_uuid(stored: &[u8], given: &[u8]) -> bool {
    stored.ct_eq(given).into()
}
