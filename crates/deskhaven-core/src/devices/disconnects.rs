//! An id is handed out once, whether or not the reply arrives: losing it costs a click, replaying
//! it could close a later connection. Only ids the device reports live in that heartbeat are handed
//! out, so a heartbeat with no `conns` (a clone, a stolen uuid) cannot swallow them. Requests are
//! short-lived: conn ids are a per-process counter, so after a client restart an old id can name a
//! new connection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::presence::MAX_CONNS;
use crate::clock::SharedClock;

pub const DISCONNECT_TTL: Duration = Duration::from_secs(60);
pub const MAX_QUEUED_DEVICES: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QueueFull {
    #[error("too many devices with pending disconnects")]
    Devices,
}

#[derive(Debug, Default)]
struct Pending {
    all: Option<i64>,
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

    /// An id already queued keeps its deadline.
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

    pub fn disconnect_all(&self, rustdesk_id: &str) -> Result<(), QueueFull> {
        self.queue(rustdesk_id, |pending, now| pending.all = Some(now))
    }

    pub fn is_pending(&self, rustdesk_id: &str) -> bool {
        self.lock().contains_key(rustdesk_id)
    }

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
