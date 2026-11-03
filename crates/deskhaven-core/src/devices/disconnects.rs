//! [`DisconnectQueue`] (02 §3, 03 §2.1): connections an administrator (or, later, an ACL
//! change) wants closed, waiting for the device's next heartbeat. The heartbeat reply lists
//! them in `disconnect` and the client closes each one it still has ("Closed manually by the
//! web console", `src/server/connection.rs:1389-1392`).
//!
//! - **Once**: an id is handed out by one heartbeat and then gone, whether or not the reply
//!   arrives. Losing it costs a click; replaying it could close a later connection.
//! - **Only live connections**: the reply carries only ids the device reports in that
//!   heartbeat's `conns`. A requested id it does not report stays queued until reported or
//!   expired, so a heartbeat with no `conns` (a clone, or a stolen uuid) cannot swallow it.
//!   A disconnect-all request is used up by the first heartbeat that reports connections.
//! - **Short-lived** ([`DISCONNECT_TTL`], per id from its own request, on the monotonic
//!   clock): connection ids are a per-process counter in the client, so after a client
//!   restart an old id can name a new connection. A request the device does not pick up in
//!   time expires; a device with live connections heartbeats every 3 s (`sync.rs:232-235`).
//!   A later request never extends the deadline of an id already queued.
//! - **Bounded**: at most [`MAX_QUEUED_DEVICES`] devices with pending requests and
//!   [`MAX_CONNS`] ids each. Not persisted (02 §3): after a restart the connections are new.

use std::collections::HashMap;
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

/// What waits for one device. Times are monotonic ms
/// ([`Clock::elapsed_ms`](crate::clock::Clock::elapsed_ms)) of the request that queued them.
#[derive(Debug, Default)]
struct Pending {
    /// A disconnect-all request.
    all: Option<i64>,
    /// Requested connection ids.
    ids: HashMap<i32, i64>,
}

impl Pending {
    fn prune(&mut self, now: i64) {
        self.all = self.all.filter(|at| !expired(*at, now));
        self.ids.retain(|_, at| !expired(*at, now));
    }

    fn is_empty(&self) -> bool {
        self.all.is_none() && self.ids.is_empty()
    }
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

    /// Asks device `rustdesk_id` to close connections `conn_ids`, added to what is already
    /// pending for it (an id already queued keeps its deadline; ids beyond [`MAX_CONNS`] per
    /// device are ignored).
    pub fn disconnect(&self, rustdesk_id: &str, conn_ids: &[i32]) -> Result<(), QueueFull> {
        self.queue(rustdesk_id, |pending, now| {
            for id in conn_ids {
                if pending.ids.len() >= MAX_CONNS && !pending.ids.contains_key(id) {
                    break;
                }
                pending.ids.entry(*id).or_insert(now);
            }
        })
    }

    /// Asks device `rustdesk_id` to close every connection reported by its next heartbeat
    /// that reports any.
    pub fn disconnect_all(&self, rustdesk_id: &str) -> Result<(), QueueFull> {
        self.queue(rustdesk_id, |pending, now| pending.all = Some(now))
    }

    /// Whether a request for `rustdesk_id` is waiting (expired ones count until the next
    /// heartbeat or request drops them).
    pub fn is_pending(&self, rustdesk_id: &str) -> bool {
        self.lock().contains_key(rustdesk_id)
    }

    /// The connection ids to put in the heartbeat reply of `rustdesk_id`, which reports the
    /// live connections `active`: the requested ones among them. Removes only what it hands
    /// out.
    pub(crate) fn take(&self, rustdesk_id: &str, active: &[i32]) -> Vec<i32> {
        let now = self.inner.clock.elapsed_ms();
        let mut table = self.lock();
        let Some(pending) = table.get_mut(rustdesk_id) else {
            return Vec::new();
        };
        pending.prune(now);
        let mut handed = Vec::new();
        for id in active {
            if handed.len() >= MAX_CONNS {
                break;
            }
            let requested = pending.ids.remove(id).is_some();
            if (requested || pending.all.is_some()) && !handed.contains(id) {
                handed.push(*id);
            }
        }
        if !handed.is_empty() {
            pending.all = None;
        }
        if pending.is_empty() {
            table.remove(rustdesk_id);
        }
        handed
    }

    fn queue(
        &self,
        rustdesk_id: &str,
        add: impl FnOnce(&mut Pending, i64),
    ) -> Result<(), QueueFull> {
        let now = self.inner.clock.elapsed_ms();
        let mut table = self.lock();
        if !table.contains_key(rustdesk_id) && table.len() >= MAX_QUEUED_DEVICES {
            table.retain(|_, p| {
                p.prune(now);
                !p.is_empty()
            });
            if table.len() >= MAX_QUEUED_DEVICES {
                return Err(QueueFull::Devices);
            }
        }
        let pending = table.entry(rustdesk_id.to_owned()).or_default();
        pending.prune(now);
        add(pending, now);
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

fn expired(queued: i64, now: i64) -> bool {
    let ttl = i64::try_from(DISCONNECT_TTL.as_millis()).unwrap_or(i64::MAX);
    now.saturating_sub(queued) >= ttl
}

#[cfg(test)]
mod tests;
