//! Only a token's SHA-256 is stored; the raw token leaves the service once. The owner check runs on
//! every database read, so it holds even when a user event was never delivered. A commit hook drops
//! the user's cache entries before the `UserService` call returns, so the very next request sees
//! the change.

mod cache;
mod create;
mod error;
mod events;
mod list;
mod model;
mod revoke;
mod revoke_own;
mod token;
mod verify;

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::{AUTH_SESSION_IDLE_TTL, AUTH_SESSION_TTL};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::{DatabaseConnection, DbErr};

pub use cache::{CACHE_TTL, CacheGeneration, CachedSession, DEFAULT_CAPACITY, SessionCache};
pub use error::SessionError;
pub use model::{
    AccountFlags, ClientDevice, DeviceBinding, MAX_DEVICE_FIELD_LEN, MAX_DEVICE_INFO_LEN, Session,
    SessionKind, SessionPrincipal,
};
pub(crate) use revoke::revoke_user_sessions;
pub use token::{SessionToken, TOKEN_BYTES, TOKEN_LEN, TokenHash};
pub use verify::CachedAuth;

use crate::clock::SharedClock;
use crate::settings::SettingsService;
use crate::users::{UserChange, UserService};

pub const TOUCH_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct SessionService {
    core: Arc<Core>,
    /// Kept out of `Core`: the event consumer holds `Core`, and the event channel closes
    /// (ending the consumer) only once every `UserService` is gone.
    users: UserService,
}

struct Core {
    db: DatabaseConnection,
    settings: SettingsService,
    cache: SessionCache,
    clock: SharedClock,
}

impl SessionService {
    /// Must be called inside a Tokio runtime. Give both services the same `clock`.
    pub fn new(
        db: DatabaseConnection,
        settings: SettingsService,
        users: UserService,
        clock: SharedClock,
    ) -> Self {
        let cache = SessionCache::default();
        let (hook_cache, flush_cache) = (cache.clone(), cache.clone());
        // Fails closed: if the targeted invalidation panics, every entry goes.
        users.register_commit_hook_with_fallback(
            Arc::new(move |change: &UserChange| {
                hook_cache.invalidate_user(change.user_id());
            }),
            Arc::new(move || flush_cache.flush()),
        );
        let core = Core {
            db,
            settings,
            cache,
            clock,
        };
        let service = Self {
            core: Arc::new(core),
            users,
        };
        let events = service.users.subscribe();
        drop(service.spawn_event_consumer(events));
        service
    }

    pub fn cache(&self) -> &SessionCache {
        &self.core.cache
    }
}

impl Core {
    fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    fn session_ttl_ms(&self) -> i64 {
        millis(self.settings.snapshot().get(&AUTH_SESSION_TTL).as_std())
    }

    fn idle_ttl_ms(&self) -> i64 {
        millis(
            self.settings
                .snapshot()
                .get(&AUTH_SESSION_IDLE_TTL)
                .as_std(),
        )
    }
}

fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

fn timestamp(ms: i64) -> Result<TimestampMs, DbErr> {
    time::from_millis(ms).ok_or_else(|| DbErr::Custom(format!("timestamp {ms} out of range")))
}

/// Dropping the caller's future can then neither interrupt a commit nor skip the cache
/// invalidation that follows it.
async fn detached<T, Fut>(op: Fut) -> Result<T, SessionError>
where
    Fut: Future<Output = Result<T, SessionError>> + Send + 'static,
    T: Send + 'static,
{
    tokio::spawn(op).await.map_err(|_| SessionError::Aborted)?
}
