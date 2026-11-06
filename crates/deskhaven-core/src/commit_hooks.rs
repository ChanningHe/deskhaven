//! Synchronous after-commit callbacks shared by the user and group services.
//!
//! The write has committed by the time hooks run, so nothing a hook does may turn it into an
//! error: a panicking hook is caught and logged, and the remaining hooks (and the caller's
//! event broadcast) still run. Hooks run on a snapshot of the list taken without holding the
//! lock while they execute, so a hook may register another hook; the new one sees the next
//! change, not the current one.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, PoisonError, RwLock};

/// One after-commit callback.
pub(crate) type Hook<C> = Arc<dyn Fn(&C) + Send + Sync>;

/// A registry of `Fn(&C)` hooks. Clones share the registry.
pub(crate) struct CommitHooks<C> {
    hooks: Arc<RwLock<Vec<Hook<C>>>>,
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
        self.hooks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hook);
    }

    /// Calls every hook registered so far with `change`, in registration order.
    pub(crate) fn run(&self, change: &C) {
        let snapshot = self
            .hooks
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for hook in snapshot {
            if catch_unwind(AssertUnwindSafe(|| hook(change))).is_err() {
                tracing::error!(?change, "commit hook panicked; the change stays committed");
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
}
