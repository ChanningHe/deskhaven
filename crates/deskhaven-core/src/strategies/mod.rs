//! Secrets in strategies reach devices in clear: secret options are sealed at rest, but sent in clear
//! to every device a strategy applies to, and a device's placement can come from its own `preset-*`
//! fields. Treat every option value as readable by any device of its targets.

mod admin;
mod assign;
mod cache;
pub mod catalog;
mod crud;
mod effective;
mod error;
mod load;
mod model;
mod options;
mod sealing;

use std::future::Future;
use std::sync::Arc;

use sea_orm::DatabaseConnection;

pub use admin::{AssignedTarget, StrategyQuery, StrategySort, StrategySortField, StrategySummary};
pub(crate) use assign::{assign_in, assignment_in, id_by_name_in};
pub use assign::{forget_device, forget_device_group, forget_group, forget_user};
pub use cache::StrategyCache;
pub use effective::{DeviceInputs, EffectiveStrategy};
pub use error::StrategyError;
pub use model::{Assignment, NewStrategy, Strategy, StrategyUpdate, Target};
pub use options::{KEY_MAX_LEN, MAX_OPTIONS, Options, OptionsPatch, VALUE_MAX_LEN};

use crate::clock::SharedClock;
use crate::crypto::SecretBox;
use crate::groups::{GroupChange, GroupService};
use crate::users::{UserChange, UserEvent, UserService};
use cache::Lookup;

#[derive(Clone)]
pub struct StrategyService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    crypto: Arc<SecretBox>,
    clock: SharedClock,
    cache: StrategyCache,
}

impl StrategyService {
    pub fn new(db: DatabaseConnection, crypto: Arc<SecretBox>, clock: SharedClock) -> Self {
        let cache = StrategyCache::new(Arc::clone(&clock));
        Self {
            inner: Arc::new(Inner {
                db,
                crypto,
                clock,
                cache,
            }),
        }
    }

    pub fn cache(&self) -> &StrategyCache {
        &self.inner.cache
    }

    pub fn watch_users(&self, users: &UserService) {
        let (cache, flush_cache) = (self.inner.cache.clone(), self.inner.cache.clone());
        users.register_commit_hook_with_fallback(
            Arc::new(move |change: &UserChange| {
                if matches!(change, UserChange::Event(UserEvent::Deleted { .. })) {
                    cache.invalidate();
                }
            }),
            Arc::new(move || flush_cache.invalidate()),
        );
    }

    pub fn watch_groups(&self, groups: &GroupService) {
        let cache = self.inner.cache.clone();
        groups.register_commit_hook(Arc::new(move |_: &GroupChange| cache.invalidate()));
    }

    /// After the first call this does not touch the database until something invalidates the cache.
    pub async fn effective(
        &self,
        device: &DeviceInputs,
    ) -> Result<EffectiveStrategy, StrategyError> {
        let snapshot = self.snapshot().await?;
        Ok(snapshot.effective(device))
    }

    /// Never touches the database. The client needs the strategy when its stored value differs (`!=`).
    pub fn modified_at(&self, device: &DeviceInputs) -> i64 {
        effective::stamp(self.inner.cache.version(), device)
    }

    async fn snapshot(&self) -> Result<Arc<effective::Snapshot>, StrategyError> {
        let cache = &self.inner.cache;
        let ticket = match cache.lookup() {
            Lookup::Hit(snapshot) => return Ok(snapshot),
            Lookup::Miss(ticket) => ticket,
        };
        let inner = &self.inner;
        let snapshot = Arc::new(load::snapshot(&inner.db, &inner.crypto, ticket.version).await?);
        cache.store(ticket, &snapshot);
        Ok(snapshot)
    }

    /// Dropping the caller's future can then neither interrupt a commit nor lose the cache invalidation
    /// that follows it.
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
