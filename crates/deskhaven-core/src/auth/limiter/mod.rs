//! Login rate limiting and account lockout (06-security §3.6).
//!
//! Two guards, combined by [`LoginThrottle`]:
//!
//! - [`IpLimiter`]: in-memory token bucket per client IPv4 address / IPv6 /64
//!   (`auth.ip_rate_limit.per_minute`); trusted-proxy fallback addresses share one larger
//!   bucket. See `ip.rs`.
//! - [`AccountLockout`]: persistent attempt counters per username, surviving restarts.
//!   Each attempt counts on two rows (`key.rs`): the username **from this source** (IPv4 /
//!   IPv6 **/48** / `fallback`), with exponential backoff up to 24h, and the username
//!   **globally**, with [`GLOBAL_FACTOR`]× the threshold and a fixed `auth.lockout.duration`.
//!   One attacker can therefore lock a username only for its own source; a distributed
//!   attack locks it for at most `auth.lockout.duration` at a time, and never for a **known
//!   source** (one that completed a login of that username within
//!   [`KNOWN_SOURCE_RETENTION`]). With `auth.lockout.window <= 4 × auth.lockout.duration`
//!   (a settings rule) a single source alone can never trip the global row.
//!   [`LoginThrottle::unlock`] clears all of a username's rows (admin unlock, M3-04). See
//!   `policy.rs` and `lockout.rs` for the exact rules.
//!
//! # Handler protocol (count before verify)
//!
//! One-step login (no second factor):
//!
//! ```text
//! let attempt = throttle.pre_check(client, username).await?;  // Throttled -> 429, no verification
//! match verify(username, password).await {
//!     Ok(user)  => throttle.on_success(attempt).await?,     // login complete
//!     Err(Busy) => { throttle.refund(attempt).await?; 503 } // never counted
//!     Err(_)    => throttle.on_failure(attempt),            // already counted; same response
//! }                                                         // for unknown accounts
//! ```
//!
//! Two-step login (password, then TOTP / email code). `on_success` is only for a **fully**
//! authenticated login: calling it after the password alone would reset the budget and let
//! a password holder alternate password and second-factor guesses forever.
//!
//! ```text
//! // step 1
//! let attempt = throttle.pre_check(client, username).await?;
//! verify(username, password) ok -> let pending = throttle.on_password_ok_pending_2fa(attempt);
//!                                  store pending.parts() in the challenge
//! // step 2 (same username key; any client)
//! let pending = PendingSubject::from_parts(..)?;
//! let attempt = throttle.pre_check(client, username).await?;
//! match verify_second_factor(..) {
//!     Ok  => throttle.complete(pending, attempt).await?,
//!     Err => throttle.on_failure(attempt),                  // counts against the same budget
//! }
//! ```
//!
//! `pre_check` reserves the attempt in the same write transaction that checks the lock, so
//! concurrent requests cannot all slip past a check made before any of them failed: per lock
//! cycle at most `max_failures` verifications run per source, password and second factor
//! together. Dropping an [`Attempt`] without reporting counts as a failure.
//!
//! Endpoints:
//! - Client `/api/login` and console login: the protocols above.
//! - 2FA verification: step 2 above, with the **same username key** as the first step.
//! - OIDC initiation: [`IpLimiter::check`] only (there is no username to count).
//! - Bearer-authenticated endpoints: a token valid in the session cache passes without
//!   touching the limiter. Otherwise (a database lookup) [`LoginThrottle::check_bearer`]
//!   first (429 while a recent bearer rejection found the IP over its limit, no database
//!   access), then [`LoginThrottle::on_bearer_rejected`] when the token does not
//!   authenticate (401). Valid tokens spend nothing. Bearer rejections and logins draw from
//!   the same bucket, but only bearer rejections close the gate, so heavy login traffic
//!   from one address never locks its users out of account features.
//!
//!   Lookup bound for random-token spraying from one network: roughly the per-minute quota
//!   plus one denied lookup per refill. It is approximate: requests already past the gate
//!   when the first denial is recorded still look their token up, so under concurrency up
//!   to one in-flight batch more gets through each time the gate reopens.
//!
//! **Proxy fallback**: requests whose address resolved to a trusted proxy
//! (`ClientAddr::is_proxy_fallback`) all share the source `fallback` — and one IP bucket — by
//! design: their real clients cannot be told apart, so one client's failures lock the
//! username for all of them. This only happens with a misconfigured proxy or a client inside
//! `trusted_proxies`, and `ip.rs` logs a rate-limited warning when it does.
//!
//! Nothing here looks the account up, so [`ThrottleError::Throttled`] is the same for
//! existing and unknown usernames. Equal timing of the verification itself (dummy Argon2)
//! is the user service's job.

mod cleanup;
mod ip;
mod key;
mod lockout;
mod policy;

use std::net::IpAddr;
use std::time::Duration;

use sea_orm::{DatabaseConnection, DbErr};

pub use cleanup::{LOCKOUT_RETENTION, UNLOCKED_RETENTION};
pub use ip::{FALLBACK_FACTOR, FALLBACK_WARN_INTERVAL, IpLimiter, SWEEP_EVERY, ip_key};
pub use key::{LOCKOUT_IPV6_PREFIX, Subject, source_prefix};
pub use lockout::{
    AccountLockout, Attempt, KNOWN_SOURCE_RETENTION, LockStatus, MAX_LOCKOUT_ROWS, PendingSubject,
    Reserved,
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

    /// The login is **fully** authenticated (no second factor pending): deletes the
    /// username's global row and makes this source a known source.
    pub async fn on_success(&self, attempt: Attempt) -> Result<(), DbErr> {
        self.accounts.release(attempt).await
    }

    /// The password was right but a second factor is still required. The attempt stays
    /// counted; store the returned subject with the challenge.
    pub fn on_password_ok_pending_2fa(&self, attempt: Attempt) -> PendingSubject {
        PendingSubject::from_subject(attempt.subject().clone())
    }

    /// The second factor (reserved as `second`) succeeded: the login is complete.
    pub async fn complete(&self, pending: PendingSubject, second: Attempt) -> Result<(), DbErr> {
        self.accounts.complete(pending, second).await
    }

    /// Verification never ran (password hasher busy -> 503): the attempt is not counted.
    pub async fn refund(&self, attempt: Attempt) -> Result<(), DbErr> {
        self.accounts.refund(attempt).await
    }

    /// Before looking a presented bearer token up in the database: `Err(retry_after)` while
    /// a bearer rejection found `client` over its per-IP limit (login denials do not count),
    /// so a token-spraying client causes no lookups. Takes no token.
    pub fn check_bearer(&self, client: ClientAddr) -> Result<(), Duration> {
        self.ip.over_limit(client).map_or(Ok(()), Err)
    }

    /// A presented bearer token did not authenticate: spends one token of `client`'s
    /// per-IP budget, the one logins draw from. `Err(retry_after)` when none was left, which
    /// also closes [`check_bearer`](Self::check_bearer) until a token frees up.
    pub fn on_bearer_rejected(&self, client: ClientAddr) -> Result<(), Duration> {
        self.ip.check_recorded(client)
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
