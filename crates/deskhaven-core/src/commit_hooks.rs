//! Synchronous after-commit callbacks shared by the user and group services.
//!
//! The write has committed by the time hooks run, so nothing a hook does may turn it into an
//! error: a panicking hook is caught and logged, and the remaining hooks (and the caller's
//! event broadcast) still run. A hook may come with a fallback that runs (also caught) when
//! it panicked, so a cache whose targeted invalidation failed can fail closed by dropping
//! everything. Hooks run on a snapshot of the list taken without holding the lock while they
//! execute, so a hook may register another hook; the new one sees the next change, not the
//! current one.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, PoisonError, RwLock};

/// One after-commit callback.
pub(crate) type Hook<C> = Arc<dyn Fn(&C) + Send + Sync>;
/// What runs instead when a hook panicked.
pub type Fallback = Arc<dyn Fn() + Send + Sync>;

type Entry<C> = (Hook<C>, Option<Fallback>);

/// A registry of `Fn(&C)` hooks. Clones share the registry.
pub(crate) struct CommitHooks<C> {
    hooks: Arc<RwLock<Vec<Entry<C>>>>,
}

impl<C> Clone for CommitHooks<C> {
    fn clone(&self) -> Self {
        Self {
            hooks: Arc::clone(&self.hooks),
        }
    }
}

impl<C> Default for CommitHooks<C> {
    fn default() -> Self {
        Self {
            hooks: Arc::default(),
        }
    }
}

impl<C: std::fmt::Debug> CommitHooks<C> {
    pub(crate) fn register(&self, hook: Hook<C>) {
        self.push((hook, None));
    }

    /// Like [`CommitHooks::register`]; `fallback` runs whenever `hook` panics.
    pub(crate) fn register_with_fallback(&self, hook: Hook<C>, fallback: Fallback) {
        self.push((hook, Some(fallback)));
    }

    fn push(&self, entry: Entry<C>) {
        self.hooks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    /// Calls every hook registered so far with `change`, in registration order.
    pub(crate) fn run(&self, change: &C) {
        let snapshot = self
            .hooks
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for (hook, fallback) in snapshot {
            if catch_unwind(AssertUnwindSafe(|| hook(change))).is_ok() {
                continue;
            }
            tracing::error!(?change, "commit hook panicked; the change stays committed");
            if let Some(fallback) = fallback
                && catch_unwind(AssertUnwindSafe(|| fallback())).is_err()
            {
                tracing::error!(?change, "commit hook fallback panicked too");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::sync::Mutex;

    use super::*;

    #[test]
    fn a_panicking_hook_is_skipped_and_the_rest_still_run_in_order() {
        let hooks = CommitHooks::<u8>::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (a, b) = (Arc::clone(&seen), Arc::clone(&seen));
        hooks.register(Arc::new(move |c: &u8| a.lock().unwrap().push(*c)));
        hooks.register(Arc::new(|_: &u8| panic!("hook bug")));
        hooks.register(Arc::new(move |c: &u8| b.lock().unwrap().push(*c + 10)));

        hooks.run(&1);
        hooks.run(&2);

        assert_eq!(*seen.lock().unwrap(), [1, 11, 2, 12]);
    }

    #[test]
    fn a_hook_registered_from_a_hook_runs_from_the_next_change() {
        let hooks = CommitHooks::<u8>::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (registry, sink) = (hooks.clone(), Arc::clone(&seen));
        hooks.register(Arc::new(move |c: &u8| {
            if *c == 1 {
                let sink = Arc::clone(&sink);
                registry.register(Arc::new(move |c: &u8| sink.lock().unwrap().push(*c)));
            }
        }));

        hooks.run(&1);
        hooks.run(&2);

        assert_eq!(*seen.lock().unwrap(), [2]);
    }

    #[test]
    fn a_fallback_runs_only_when_its_hook_panics_and_may_panic_itself() {
        let hooks = CommitHooks::<u8>::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (a, b, c) = (Arc::clone(&seen), Arc::clone(&seen), Arc::clone(&seen));
        hooks.register_with_fallback(
            Arc::new(|_: &u8| panic!("hook bug")),
            Arc::new(move || a.lock().unwrap().push("fallback")),
        );
        hooks.register_with_fallback(
            Arc::new(move |_: &u8| b.lock().unwrap().push("calm")),
            Arc::new(|| panic!("never runs")),
        );
        hooks.register_with_fallback(
            Arc::new(|_: &u8| panic!("hook bug")),
            Arc::new(|| panic!("fallback bug")),
        );
        hooks.register(Arc::new(move |_: &u8| c.lock().unwrap().push("last")));

        hooks.run(&1);

        assert_eq!(*seen.lock().unwrap(), ["fallback", "calm", "last"]);
    }
}
