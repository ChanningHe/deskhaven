//! Login rate limiting and account lockout (06-security §3.6).
//!
//! Two guards, combined by [`LoginThrottle`]:
//!
//! - [`IpLimiter`]: in-memory token bucket per client IPv4 address / IPv6 /64
//!   (`auth.ip_rate_limit.per_minute`); trusted-proxy fallback addresses share one larger
//!   bucket. See `ip.rs`.
//! - [`AccountLockout`]: persistent attempt counters per username, surviving restarts.
//!   Each attempt counts on two rows (`key.rs`): the username **from this source** (IPv4 /
//!   IPv6 /64 / `fallback`), with exponential backoff up to 24h, and the username
//!   **globally**, with 4× the threshold and a fixed `auth.lockout.duration`. One attacker
//!   can therefore lock a username only for its own source; a distributed attack locks it
//!   for at most `auth.lockout.duration` at a time. [`LoginThrottle::unlock`] clears all of
//!   a username's rows (admin unlock, M3-04). See `policy.rs` for the exact rules.
//!
//! # Handler protocol (count before verify)
//!
//! ```text
//! let attempt = throttle.pre_check(client, username).await?;  // Throttled -> 429, no verification
//! match verify(username, password).await {
//!     Ok(user)  => throttle.on_success(attempt).await?,     // clears this source + global rows
//!     Err(Busy) => { throttle.refund(attempt).await?; 503 } // never counted
//!     Err(_)    => throttle.on_failure(attempt),            // already counted; same response
//! }                                                         // for unknown accounts
//! ```
//!
//! `pre_check` reserves the attempt in the same write transaction that checks the lock, so
//! concurrent requests cannot all slip past a check made before any of them failed: per lock
//! cycle at most `max_failures` verifications run per source. Dropping the [`Attempt`]
//! without reporting counts as a failure.
//!
//! Endpoints:
//! - Client `/api/login` and console login: the full protocol above.
//! - 2FA verification: the same protocol with the **same username key** as the first step,
//!   so second-factor guessing shares the password's budget.
//! - OIDC initiation: [`IpLimiter::check`] only (there is no username to count).
//!
//! Nothing here looks the account up, so [`ThrottleError::Throttled`] is the same for
//! existing and unknown usernames. Equal timing of the verification itself (dummy Argon2)
//! is the user service's job.

mod ip;
mod key;
mod lockout;
mod policy;

use std::net::IpAddr;
use std::time::Duration;

use sea_orm::{DatabaseConnection, DbErr};

pub use ip::{FALLBACK_FACTOR, FALLBACK_WARN_INTERVAL, IpLimiter, SWEEP_EVERY, ip_key};
pub use key::Subject;
pub use lockout::{
    AccountLockout, Attempt, LOCKOUT_RETENTION, LockStatus, Reserved, UNLOCKED_RETENTION,
};
pub use policy::{GLOBAL_FACTOR, LEVEL_DECAY, MAX_LOCK};

use crate::clock::SharedClock;
use crate::settings::SettingsService;

/// The client address as resolved by the HTTP layer (`deskhaven_http::ClientIp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddr {
    pub addr: IpAddr,
    /// `addr` is a trusted proxy, not the client: limited in the shared fallback bucket and
    /// counted under source `fallback`.
    pub is_proxy_fallback: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ThrottleError {
    /// Too many attempts from this client or for this username. Deliberately does not say
    /// which, nor whether the account exists.
    #[error("too many login attempts; retry after {retry_after:?}")]
    Throttled { retry_after: Duration },
    #[error(transparent)]
    Db(#[from] DbErr),
}

/// Rate limiting and lockout for login handlers. One per process (the IP buckets live in
/// memory), shared through `AppState`.
pub struct LoginThrottle {
    ip: IpLimiter,
    accounts: AccountLockout,
}

impl LoginThrottle {
    pub fn new(db: DatabaseConnection, settings: SettingsService, clock: SharedClock) -> Self {
        Self {
            ip: IpLimiter::new(settings.clone(), clock.clone()),
            accounts: AccountLockout::new(db, settings, clock),
        }
    }

    /// Before verifying: takes one IP token, then reserves (counts) one attempt for
    /// `username` from `client`, unless it is locked.
    pub async fn pre_check(
        &self,
        client: ClientAddr,
        username: &str,
    ) -> Result<Attempt, ThrottleError> {
        self.ip
            .check(client)
            .map_err(|retry_after| ThrottleError::Throttled { retry_after })?;
        match self
            .accounts
            .reserve(&Subject::new(client, username))
            .await?
        {
            Reserved::Granted(attempt) => Ok(attempt),
            Reserved::Locked { retry_after } => Err(ThrottleError::Throttled { retry_after }),
        }
    }

    /// Verification failed (wrong password, unknown or disabled account alike). The attempt
    /// is already counted; returns whether it started a lock (for login events).
    pub fn on_failure(&self, attempt: Attempt) -> LockStatus {
        attempt.status()
    }

    /// Verification succeeded: clears the username's counters for this source and globally.
    pub async fn on_success(&self, attempt: Attempt) -> Result<(), DbErr> {
        self.accounts.release(attempt).await
    }

    /// Verification never ran (password hasher busy -> 503): the attempt is not counted.
    pub async fn refund(&self, attempt: Attempt) -> Result<(), DbErr> {
        self.accounts.refund(attempt).await
    }

    /// Admin unlock: clears every counter and lock of `username`; returns rows removed.
    pub async fn unlock(&self, username: &str) -> Result<u64, DbErr> {
        self.accounts.unlock(username).await
    }

    /// Periodic maintenance (M2-20): drops refilled IP buckets and stale lockout rows.
    /// Returns the number of lockout rows deleted.
    pub async fn cleanup(&self) -> Result<u64, DbErr> {
        self.ip.sweep();
        self.accounts.cleanup().await
    }

    pub fn ip(&self) -> &IpLimiter {
        &self.ip
    }

    pub fn accounts(&self) -> &AccountLockout {
        &self.accounts
    }
}
