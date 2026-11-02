//! The strategy cache and the version (`modified_at`) clients compare.
//!
//! One immutable [`Snapshot`] of every assignment (with the options it points at and the
//! members of assigned user groups) is loaded on first use and served until anything that
//! can change an effective strategy happens: strategy or assignment writes, deletion of a
//! user, group, device group or device, membership changes. Each of those calls
//! [`StrategyCache::invalidate`] after its transaction committed. Computing the effective
//! strategy of a device is then a few hash lookups and no database access, which is what the
//! 15 s heartbeat of every device needs.
//!
//! **Version.** The cache keeps one counter in unix milliseconds. It starts at the clock
//! reading when the cache is created (a restart counts as a change, so clients refetch once)
//! and every invalidation moves it to `max(now, previous + 1)`: strictly increasing even
//! when the clock does not move, and never lower than a value already handed out. Every
//! device's [`EffectiveStrategy::modified_at`] is this counter (or, if later, the device's
//! own `changed_at`). One global counter over-notifies (a change to one group's strategy
//! makes every device refetch once) but can never miss a change, including removals, which a
//! "newest input timestamp" cannot see. Edits made by another process are not noticed.
//!
//! **Fill race.** A reader that missed takes a [`Ticket`] (generation and version) before
//! it reads the database and stores its snapshot only if the generation is unchanged, so an
//! invalidation can never be overwritten by a snapshot read before it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::effective::Snapshot;
use crate::clock::SharedClock;

/// Shared handle; clones see the same snapshot and version.
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
    /// Unix ms; see the module documentation.
    version: i64,
}

/// What a reader that missed must present to [`StrategyCache::store`].
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

    /// Drops the snapshot and advances the version. Call after any committed change that can
    /// alter the effective strategy of some device. Cheap; safe to call spuriously.
    pub fn invalidate(&self) {
        let now = self.inner.clock.now_ms();
        let mut state = self.lock();
        state.generation += 1;
        state.snapshot = None;
        state.version = now.max(state.version.saturating_add(1));
    }

    /// The current version (unix ms): what `modified_at` of a device without its own
    /// `changed_at` is.
    pub fn version(&self) -> i64 {
        self.lock().version
    }

    /// Whether a snapshot is currently held (`false` right after an invalidation).
    pub fn is_loaded(&self) -> bool {
        self.lock().snapshot.is_some()
    }

    /// How many snapshots were read from the database so far.
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

    /// Keeps `snapshot` unless an invalidation happened since `ticket` was taken.
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
