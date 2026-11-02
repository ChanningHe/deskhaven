//! Login rate limiting and account lockout (06-security §3.6).
//!
//! Two independent guards, combined by [`LoginThrottle`] for the login-type endpoints
//! (client `/api/login`, console login, 2FA verification, OIDC start):
//!
//! - [`IpLimiter`]: in-memory token bucket per client IPv4 address / IPv6 /64
//!   (`auth.ip_rate_limit.per_minute`). See `ip.rs`.
//! - [`AccountLockout`]: persistent failure counter per username with exponential-backoff
//!   lockout (`auth.lockout.*`), surviving restarts. See `policy.rs` for the exact rule.
//!
//! Handler protocol:
//!
//! ```text
//! throttle.pre_check(client, username)?;   // Throttled -> 429-style response, no auth attempt
//! match authenticate(username, password) {
//!     Ok(user)  => throttle.on_success(username),
//!     Err(_)    => throttle.on_failure(username),   // same response for unknown accounts
//! }
//! ```
//!
//! Nothing here looks the account up, so [`ThrottleError::Throttled`] is the same for
//! existing and unknown usernames. Equal timing of the authentication itself (dummy
//! Argon2 verification) is the user service's job.

mod ip;
mod lockout;
mod policy;

use std::net::IpAddr;
use std::time::Duration;

use sea_orm::{DatabaseConnection, DbErr};

pub use ip::{IpLimiter, SWEEP_EVERY, ip_key};
pub use lockout::{AccountLockout, LOCKOUT_RETENTION, LockStatus, LockoutKey};
pub use policy::{LEVEL_DECAY, MAX_LOCK};

use crate::clock::SharedClock;
use crate::settings::SettingsService;

/// The client address as resolved by the HTTP layer (`deskhaven_http::ClientIp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddr {
    pub addr: IpAddr,
    /// `addr` is a trusted proxy, not the client: it must not be IP-limited.
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

    /// Before authenticating: takes one IP token, then checks the username's lockout.
    pub async fn pre_check(&self, client: ClientAddr, username: &str) -> Result<(), ThrottleError> {
        self.ip
            .check(client)
            .map_err(|retry_after| ThrottleError::Throttled { retry_after })?;
        match self.accounts.check(&LockoutKey::username(username)).await? {
            LockStatus::Allowed => Ok(()),
            LockStatus::Locked { retry_after } => Err(ThrottleError::Throttled { retry_after }),
        }
    }

    /// After a failed authentication (wrong password, unknown or disabled account alike).
    pub async fn on_failure(&self, username: &str) -> Result<LockStatus, DbErr> {
        self.accounts
            .record_failure(&LockoutKey::username(username))
            .await
    }

    /// After a successful authentication: resets the username's failures and backoff.
    pub async fn on_success(&self, username: &str) -> Result<(), DbErr> {
        self.accounts
            .record_success(&LockoutKey::username(username))
            .await
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
