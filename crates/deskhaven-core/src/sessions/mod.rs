//! Client and console sessions (06 §3.2, 04 §3 `sessions`).
//!
//! - **Tokens**: 256-bit opaque values from the OS CSPRNG, base64url unpadded; only their
//!   SHA-256 (hex) is stored. The raw token leaves the service once, at creation.
//! - **Expiry**: absolute (`expires_at`, from `auth.session_ttl` at creation) and idle
//!   (`last_used_at` + the current `auth.session_idle_ttl`). `last_used_at` is written at
//!   most once per [`TOUCH_INTERVAL`] per session, so the idle timeout is exact to within
//!   that interval and steady traffic causes no write per request.
//! - **Validity at read time**: a session authenticates only for the expected
//!   [`SessionKind`] and while it is unrevoked, unexpired and its owner exists and is
//!   `active`. The owner check runs on every database read, so it holds even when a
//!   [`UserEvent`](crate::users::UserEvent) was never delivered (events are in memory only);
//!   a cached session is re-read at the latest after [`CACHE_TTL`].
//! - **Revocation**: `revoke*` write `revoked_at` and invalidate the cache from a task the
//!   caller cannot cancel half-way. Password changes and disabling revoke every session of
//!   the user in the same transaction as the change (`users::write`); account deletion
//!   cascades. A commit hook drops the user's cache entries (also on role and status
//!   changes) before the `UserService` call returns, so the very next request sees the
//!   change. The [`UserEvent`](crate::users::UserEvent) consumer then invalidates and
//!   revokes again (idempotent); a lagged consumer flushes the whole cache.
//! - **Login race**: creation re-checks the
//!   [`CredentialStamp`](crate::users::CredentialStamp) of the login inside its own write
//!   transaction (see [`UserService::credential_stamp_matches`]).

mod cache;
mod create;
mod error;
mod events;
mod model;
mod revoke;
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

/// Minimum time between two `last_used_at` writes of one session.
pub const TOUCH_INTERVAL: Duration = Duration::from_secs(60);

/// Issues, validates and revokes sessions. Cheap to clone (shared state).
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
    /// Registers a commit hook on `users` that drops the changed user's cache entries before
    /// the writing call returns, subscribes to `users`' events and spawns their consumer, so
    /// it must be called inside a Tokio runtime. The consumer ends when the event channel
    /// closes. Give both services the same `clock`.
    pub fn new(
        db: DatabaseConnection,
        settings: SettingsService,
        users: UserService,
        clock: SharedClock,
    ) -> Self {
        let cache = SessionCache::default();
        let hook_cache = cache.clone();
        users.register_commit_hook(Arc::new(move |change: &UserChange| {
            hook_cache.invalidate_user(change.user_id());
        }));
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

    /// The validated-session cache (shared with every clone of this service).
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

/// Runs `op` to completion in its own task: dropping the caller's future can then neither
/// interrupt a commit nor skip the cache invalidation that follows it.
async fn detached<T, Fut>(op: Fut) -> Result<T, SessionError>
where
    Fut: Future<Output = Result<T, SessionError>> + Send + 'static,
    T: Send + 'static,
{
    tokio::spawn(op).await.map_err(|_| SessionError::Aborted)?
}
