//! Every change that can alter an effective strategy invalidates after commit, so a heartbeat costs
//! a few hash lookups and no database access.
//!
//! One global version over-notifies but can never miss a change, removals included. It is
//! in-memory: after a restart on a slower wall clock it can be lower than a client's, so clients
//! are served on `server != client`, never `server > client`. A fill stores its snapshot only if no
//! invalidation happened since its ticket.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::effective::Snapshot;
use crate::clock::SharedClock;

#[derive(Clone)]
pub struct StrategyCache {
    inner: Arc<Inner>,
}

struct Inner {
    clock: SharedClock,
    state: Mutex<State>,
    loads: AtomicU64,
}

struct State {
    snapshot: Option<Arc<Snapshot>>,
    generation: u64,
    version: i64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Ticket {
    generation: u64,
    pub(super) version: i64,
}

pub(super) enum Lookup {
    Hit(Arc<Snapshot>),
    Miss(Ticket),
}

impl StrategyCache {
    pub fn new(clock: SharedClock) -> Self {
        let state = State {
            snapshot: None,
            generation: 0,
            version: clock.now_ms(),
        };
        Self {
            inner: Arc::new(Inner {
                clock,
                state: Mutex::new(state),
                loads: AtomicU64::new(0),
            }),
        }
    }

    /// Cheap; safe to call spuriously.
    pub fn invalidate(&self) {
        let now = self.inner.clock.now_ms();
        let mut state = self.lock();
        state.generation += 1;
        state.snapshot = None;
        state.version = now.max(state.version.saturating_add(1));
    }

    pub fn version(&self) -> i64 {
        self.lock().version
    }

    pub fn is_loaded(&self) -> bool {
        self.lock().snapshot.is_some()
    }

    pub fn loads(&self) -> u64 {
        self.inner.loads.load(Ordering::Relaxed)
    }

    pub(super) fn lookup(&self) -> Lookup {
        let state = self.lock();
        match &state.snapshot {
            Some(snapshot) => Lookup::Hit(Arc::clone(snapshot)),
            None => Lookup::Miss(Ticket {
                generation: state.generation,
                version: state.version,
            }),
        }
    }

    pub(super) fn store(&self, ticket: Ticket, snapshot: &Arc<Snapshot>) {
        self.inner.loads.fetch_add(1, Ordering::Relaxed);
        let mut state = self.lock();
        if state.generation == ticket.generation {
            state.snapshot = Some(Arc::clone(snapshot));
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::clock::ManualClock;

    const T0_MS: i64 = 1_000_000;

    fn cache() -> (StrategyCache, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::at(T0_MS));
        (StrategyCache::new(clock.clone()), clock)
    }

    fn miss(cache: &StrategyCache) -> Ticket {
        match cache.lookup() {
            Lookup::Miss(ticket) => ticket,
            Lookup::Hit(_) => panic!("expected a miss"),
        }
    }

    #[test]
    fn a_snapshot_read_before_an_invalidation_is_not_kept() {
        let (cache, _) = cache();
        let ticket = miss(&cache);

        cache.invalidate();
        cache.store(ticket, &Arc::new(Snapshot::default()));

        assert!(!cache.is_loaded());
        assert!(matches!(cache.lookup(), Lookup::Miss(_)));
    }

    #[test]
    fn a_snapshot_read_after_the_last_invalidation_is_kept_and_served() {
        let (cache, _) = cache();
        cache.invalidate();
        let ticket = miss(&cache);

        cache.store(ticket, &Arc::new(Snapshot::default()));

        assert!(cache.is_loaded());
        assert!(matches!(cache.lookup(), Lookup::Hit(_)));
        assert_eq!(cache.loads(), 1);
    }

    #[test]
    fn invalidating_drops_a_kept_snapshot() {
        let (cache, _) = cache();
        cache.store(miss(&cache), &Arc::new(Snapshot::default()));
        assert!(cache.is_loaded());

        cache.invalidate();

        assert!(!cache.is_loaded());
    }

    #[test]
    fn a_stale_ticket_cannot_overwrite_a_newer_snapshot() {
        let (cache, _) = cache();
        let stale = miss(&cache);
        cache.invalidate();
        let fresh = miss(&cache);
        let kept = Arc::new(Snapshot::default());
        cache.store(fresh, &kept);

        cache.store(stale, &Arc::new(Snapshot::default()));

        match cache.lookup() {
            Lookup::Hit(held) => assert!(Arc::ptr_eq(&held, &kept)),
            Lookup::Miss(_) => panic!("snapshot lost"),
        }
    }

    #[test]
    fn the_version_starts_at_the_clock_and_only_grows() {
        let (cache, clock) = cache();
        assert_eq!(cache.version(), T0_MS);

        cache.invalidate();
        assert_eq!(cache.version(), T0_MS + 1);
        clock.advance(Duration::from_secs(1));
        cache.invalidate();
        assert_eq!(cache.version(), T0_MS + 1_000);
        clock.set(T0_MS - 10_000);
        cache.invalidate();
        assert_eq!(cache.version(), T0_MS + 1_001);
    }
}
