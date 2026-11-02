//! Client strategies (04 §6, 05 §4.3): named sets of client options that the heartbeat
//! pushes to devices.
//!
//! - **CRUD** ([`StrategyService`]): a strategy is a unique name and an [`Options`] map.
//! - **Assignment** ([`Target`]): a strategy applies to `global`, a user group, a user, a
//!   device group or a device. Each target has at most one strategy; assigning replaces.
//! - **Effective strategy** ([`StrategyService::effective`]): the strategies that apply to a
//!   device overlaid from lowest to highest priority, see `effective` for the exact rules.
//! - **Cache and version** ([`StrategyCache`]): effective strategies are computed from one
//!   in-memory snapshot, and `modified_at` is a counter that only grows.
//!
//! Anything that deletes a user, user group, device group or device removes the strategy
//! assignments of that target in the same transaction (`forget_*`) and invalidates the cache
//! after commit. For users and groups [`StrategyService::watch_users`] and
//! [`StrategyService::watch_groups`] do the second half. The device registry (M2-12) calls
//! `forget_device` and `cache().invalidate()` itself, and `invalidate()` again whenever it
//! changes a device's owner or device group (or stamps `changed_at` in [`DeviceInputs`]).

mod assign;
mod cache;
mod crud;
mod effective;
mod error;
mod load;
mod model;
mod options;

use std::future::Future;
use std::sync::Arc;

use sea_orm::DatabaseConnection;

pub use assign::{forget_device, forget_device_group, forget_group, forget_user};
pub use cache::StrategyCache;
pub use effective::{DeviceInputs, EffectiveStrategy};
pub use error::StrategyError;
pub use model::{Assignment, NewStrategy, Strategy, StrategyUpdate, Target};
pub use options::{KEY_MAX_LEN, MAX_OPTIONS, Options, VALUE_MAX_LEN};

use crate::clock::SharedClock;
use crate::groups::{GroupChange, GroupService};
use crate::users::{UserChange, UserEvent, UserService};
use cache::Lookup;

/// Manages strategies, their assignments and the effective strategy of devices. Cheap to
/// clone (shared state).
#[derive(Clone)]
pub struct StrategyService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    clock: SharedClock,
    cache: StrategyCache,
}

impl StrategyService {
    /// Timestamps and the strategy version come from `clock` (share it with the other
    /// services).
    pub fn new(db: DatabaseConnection, clock: SharedClock) -> Self {
        let cache = StrategyCache::new(Arc::clone(&clock));
        Self {
            inner: Arc::new(Inner { db, clock, cache }),
        }
    }

    pub fn cache(&self) -> &StrategyCache {
        &self.inner.cache
    }

    /// Invalidates the cache when a user is deleted (its device ownership is cleared and
    /// its assignment removed by `UserService::delete`).
    pub fn watch_users(&self, users: &UserService) {
        let cache = self.inner.cache.clone();
        users.register_commit_hook(Arc::new(move |change: &UserChange| {
            if matches!(change, UserChange::Event(UserEvent::Deleted { .. })) {
                cache.invalidate();
            }
        }));
    }

    /// Invalidates the cache when a group is deleted or its membership changes.
    pub fn watch_groups(&self, groups: &GroupService) {
        let cache = self.inner.cache.clone();
        groups.register_commit_hook(Arc::new(move |_: &GroupChange| cache.invalidate()));
    }

    /// The strategy a device ends up with, merged from everything assigned to it, with the
    /// version to send to the client. After the first call this does not touch the database
    /// until something invalidates the cache.
    pub async fn effective(
        &self,
        device: &DeviceInputs,
    ) -> Result<EffectiveStrategy, StrategyError> {
        let snapshot = self.snapshot().await?;
        Ok(snapshot.effective(device))
    }

    /// `modified_at` of [`effective`](Self::effective) without computing it, for the
    /// heartbeat's "does this client need the strategy" check. Never touches the database.
    pub fn modified_at(&self, device: &DeviceInputs) -> i64 {
        effective::stamp(self.inner.cache.version(), device)
    }

    async fn snapshot(&self) -> Result<Arc<effective::Snapshot>, StrategyError> {
        let cache = &self.inner.cache;
        let ticket = match cache.lookup() {
            Lookup::Hit(snapshot) => return Ok(snapshot),
            Lookup::Miss(ticket) => ticket,
        };
        let snapshot = Arc::new(load::snapshot(&self.inner.db, ticket.version).await?);
        cache.store(ticket, &snapshot);
        Ok(snapshot)
    }

    /// Runs `op` to completion in its own task: dropping the caller's future can then
    /// neither interrupt a commit nor lose the cache invalidation that follows it.
    async fn detached<T, F, Fut>(&self, op: F) -> Result<T, StrategyError>
    where
        F: FnOnce(Arc<Inner>) -> Fut,
        Fut: Future<Output = Result<T, StrategyError>> + Send + 'static,
        T: Send + 'static,
    {
        tokio::spawn(op(Arc::clone(&self.inner)))
            .await
            .map_err(|_| StrategyError::Aborted)?
    }
}
