//! One attacker can lock a username only for its own source; a distributed attack locks it for
//! at most `auth.lockout.duration` at a time, and never for a known source.
//!
//! Count before verify: `pre_check` reserves the attempt in the transaction that checks the
//! lock, so concurrent requests cannot all slip past it. `on_success` is only for a fully
//! authenticated login: calling it after the password alone would reset the budget and let a
//! password holder alternate password and second-factor guesses forever.
//!
//! Bearer rejections and logins draw from the same IP bucket, but only bearer rejections close
//! the bearer gate, so heavy login traffic never locks users out of account features.
//!
//! Proxy-fallback requests all share the source `fallback` by design: their real clients cannot
//! be told apart. Nothing here looks the account up, so [`ThrottleError::Throttled`] is the
//! same for existing and unknown usernames.

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
    Reserved, Unlocked, unlock_guarded,
};
pub use policy::{GLOBAL_FACTOR, LEVEL_DECAY, MAX_LOCK};

use crate::clock::SharedClock;
use crate::settings::SettingsService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAddr {
    pub addr: IpAddr,
    pub is_proxy_fallback: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ThrottleError {
    /// Deliberately does not say which, nor whether the account exists.
    #[error("too many login attempts; retry after {retry_after:?}")]
    Throttled { retry_after: Duration },
    #[error(transparent)]
    Db(#[from] DbErr),
}

/// One per process: the IP buckets live in memory.
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

    pub fn on_failure(&self, attempt: Attempt) -> LockStatus {
        attempt.status()
    }

    pub async fn on_success(&self, attempt: Attempt) -> Result<(), DbErr> {
        self.accounts.release(attempt).await
    }

    /// The attempt stays counted; store the returned subject with the challenge.
    pub fn on_password_ok_pending_2fa(&self, attempt: Attempt) -> PendingSubject {
        PendingSubject::from_subject(attempt.subject().clone())
    }

    pub async fn complete(&self, pending: PendingSubject, second: Attempt) -> Result<(), DbErr> {
        self.accounts.complete(pending, second).await
    }

    pub async fn refund(&self, attempt: Attempt) -> Result<(), DbErr> {
        self.accounts.refund(attempt).await
    }

    /// Login denials do not count, so a token-spraying client causes no lookups.
    pub fn check_bearer(&self, client: ClientAddr) -> Result<(), Duration> {
        self.ip.over_limit(client).map_or(Ok(()), Err)
    }

    pub fn on_bearer_rejected(&self, client: ClientAddr) -> Result<(), Duration> {
        self.ip.check_recorded(client)
    }

    pub async fn unlock(&self, username: &str) -> Result<u64, DbErr> {
        self.accounts.unlock(username).await
    }

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
