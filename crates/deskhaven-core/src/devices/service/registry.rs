//! The signal server keeps registrations in memory and writes them behind, so a change here must
//! not contradict a registration it has not stored yet, and must be re-read by it afterwards.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use subtle::ConstantTimeEq;

use super::{DeletedHook, DeviceService, Inner};
use crate::auth::limiter::ClientAddr;
use crate::console_audit::Actor;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Requester {
    pub actor: Actor,
    pub client: ClientAddr,
}

/// No answer is no green light: a registration not stored yet could be lost to the deploy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveUnknown;

pub type LiveLookup<'a> =
    Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, LiveUnknown>> + Send + 'a>>;

pub trait LiveRegistrations: Send + Sync {
    /// `Ok(None)` when it holds none or is not running (it loads the database when it starts).
    fn uuid_of<'a>(&'a self, rustdesk_id: &'a str) -> LiveLookup<'a>;
}

#[derive(Default)]
pub(super) struct SignalRegistry {
    changed: RwLock<Vec<DeletedHook>>,
    live: RwLock<Option<Arc<dyn LiveRegistrations>>>,
}

impl DeviceService {
    pub fn on_registry_changed(&self, hook: DeletedHook) {
        let registry = &self.inner.registry;
        let mut hooks = registry
            .changed
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        hooks.push(hook);
    }

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

/// Tokens travel in scripts and prove nothing about the machine.
pub(super) fn same_uuid(stored: &[u8], given: &[u8]) -> bool {
    stored.ct_eq(given).into()
}
