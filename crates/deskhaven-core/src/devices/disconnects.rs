//! [`DisconnectQueue`] (02 §3, 03 §2.1): connections an administrator (or, later, an ACL
//! change) wants closed, waiting for the device's next heartbeat. The heartbeat reply lists
//! them in `disconnect` and the client closes each one it still has ("Closed manually by web
//! console", `src/server/connection.rs:1390`).
//!
//! - **Once**: a request is handed out by one heartbeat and then gone, whether or not the
//!   reply arrives. Losing it costs a click; replaying it could close a later connection.
//! - **Only live connections**: the reply carries only ids the device reports in that
//!   heartbeat's `conns`; requested ids it no longer reports are dropped (the connection
//!   already ended).
//! - **Short-lived** ([`DISCONNECT_TTL`]): connection ids are a per-process counter in the
//!   client, so after a client restart an old id can name a new connection. A request the
//!   device does not pick up in time expires; a device with live connections heartbeats every
//!   3 s (`sync.rs:232-235`).
//! - **Bounded**: at most [`MAX_QUEUED_DEVICES`] devices with pending requests and
//!   [`MAX_CONNS`] ids each. Not persisted (02 §3): after a restart the connections are new.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::presence::MAX_CONNS;
use crate::clock::SharedClock;

/// How long a request waits for the device's next heartbeat.
pub const DISCONNECT_TTL: Duration = Duration::from_secs(60);
/// Devices with pending requests at most.
pub const MAX_QUEUED_DEVICES: usize = 10_000;

/// Why [`DisconnectQueue`] did not queue a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueFull {
    #[error("too many devices with pending disconnects")]
    Devices,
}

/// What to close on one device.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Request {
    /// Every connection the next heartbeat reports.
    All,
    /// These connection ids, if still reported.
    Conns(BTreeSet<i32>),
}

#[derive(Debug)]
struct Pending {
    request: Request,
    /// Unix ms of the latest request (a newer one extends the deadline).
    queued_ms: i64,
}

/// Shared handle; clones see the same queue.
#[derive(Clone)]
pub struct DisconnectQueue {
    inner: Arc<Inner>,
}

struct Inner {
    clock: SharedClock,
    pending: Mutex<HashMap<String, Pending>>,
}

impl DisconnectQueue {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            inner: Arc::new(Inner {
                clock,
                pending: Mutex::default(),
            }),
        }
    }

    /// Asks device `rustdesk_id` to close connections `conn_ids` (merged with what is already
    /// pending for it; ids beyond [`MAX_CONNS`] are ignored).
    pub fn disconnect(&self, rustdesk_id: &str, conn_ids: &[i32]) -> Result<(), QueueFull> {
        self.queue(rustdesk_id, |previous| {
            let mut ids = match previous {
                Some(Request::All) => return Request::All,
                Some(Request::Conns(ids)) => ids,
                None => BTreeSet::new(),
            };
            for id in conn_ids {
                if ids.len() >= MAX_CONNS {
                    break;
                }
                ids.insert(*id);
            }
            Request::Conns(ids)
        })
    }

    /// Asks device `rustdesk_id` to close every connection its next heartbeat reports.
    pub fn disconnect_all(&self, rustdesk_id: &str) -> Result<(), QueueFull> {
        self.queue(rustdesk_id, |_| Request::All)
    }

    /// Whether a request for `rustdesk_id` is waiting (expired ones count until the next
    /// heartbeat or prune drops them).
    pub fn is_pending(&self, rustdesk_id: &str) -> bool {
        self.lock().contains_key(rustdesk_id)
    }

    /// The connection ids to put in the heartbeat reply of `rustdesk_id`, which reports the
    /// live connections `active`; removes its request.
    pub(crate) fn take(&self, rustdesk_id: &str, active: &[i32]) -> Vec<i32> {
        let now = self.inner.clock.now_ms();
        let Some(pending) = self.lock().remove(rustdesk_id) else {
            return Vec::new();
        };
        if expired(&pending, now) {
            return Vec::new();
        }
        let wanted = |id: &i32| match &pending.request {
            Request::All => true,
            Request::Conns(ids) => ids.contains(id),
        };
        let mut seen = BTreeSet::new();
        let live = active
            .iter()
            .copied()
            .filter(|id| wanted(id) && seen.insert(*id));
        live.take(MAX_CONNS).collect()
    }

    fn queue(
        &self,
        rustdesk_id: &str,
        merge: impl FnOnce(Option<Request>) -> Request,
    ) -> Result<(), QueueFull> {
        let now = self.inner.clock.now_ms();
        let mut pending = self.lock();
        let previous = pending
            .remove(rustdesk_id)
            .filter(|p| !expired(p, now))
            .map(|p| p.request);
        if previous.is_none() && pending.len() >= MAX_QUEUED_DEVICES {
            pending.retain(|_, p| !expired(p, now));
            if pending.len() >= MAX_QUEUED_DEVICES {
                return Err(QueueFull::Devices);
            }
        }
        let request = merge(previous);
        pending.insert(
            rustdesk_id.to_owned(),
            Pending {
                request,
                queued_ms: now,
            },
        );
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        // Map operations cannot leave it inconsistent.
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn expired(pending: &Pending, now: i64) -> bool {
    let ttl = i64::try_from(DISCONNECT_TTL.as_millis()).unwrap_or(i64::MAX);
    now.saturating_sub(pending.queued_ms) >= ttl
}

#[cfg(test)]
mod tests;
