//! Helpers for the `AccountLockout` tests.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use deskhaven_core::auth::limiter::{
    AccountLockout, Attempt, ClientAddr, LockStatus, Reserved, Subject,
};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::settings::SettingsService;
use deskhaven_db::entity::auth_lockout;
use deskhaven_db::testing::TestDb;
use sea_orm::{EntityTrait, PaginatorTrait};
use serde_json::json;

use super::{MINUTE, clock, set, settings};

pub const PARALLEL: usize = 20;
pub const IP_A: &str = "192.0.2.1";
pub const IP_B: &str = "198.51.100.1";

pub struct Fixture {
    pub settings: SettingsService,
    pub clock: Arc<ManualClock>,
    pub lockout: AccountLockout,
}

/// `max_failures`, window 15m, base duration 15m.
pub async fn fixture(db: &TestDb, max_failures: u32) -> Fixture {
    let settings = settings(db).await;
    set(&settings, "auth.lockout.max_failures", json!(max_failures)).await;
    let clock = clock();
    let lockout = AccountLockout::new(db.conn.clone(), settings.clone(), clock.clone());
    Fixture {
        settings,
        clock,
        lockout,
    }
}

pub fn client(addr: &str) -> ClientAddr {
    ClientAddr {
        addr: addr.parse::<IpAddr>().unwrap(),
        is_proxy_fallback: false,
    }
}

pub fn subject(name: &str, addr: &str) -> Subject {
    Subject::new(client(addr), name)
}

/// What a login handler sees for one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Verification may run; `Locked` if this attempt started a lock.
    Granted(LockStatus),
    Throttled(Duration),
}

pub fn outcome(reserved: Reserved) -> Outcome {
    match reserved {
        Reserved::Granted(attempt) => Outcome::Granted(attempt.status()),
        Reserved::Locked { retry_after } => Outcome::Throttled(retry_after),
    }
}

pub fn starts_lock(minutes: u32) -> Outcome {
    Outcome::Granted(LockStatus::Locked {
        retry_after: MINUTE * minutes,
    })
}

pub fn throttled(minutes: u32) -> Outcome {
    Outcome::Throttled(MINUTE * minutes)
}

pub const GRANTED: Outcome = Outcome::Granted(LockStatus::Allowed);

/// `n` failed attempts (reserved, attempt dropped); the last outcome.
pub async fn fail(lockout: &AccountLockout, subject: &Subject, n: usize) -> Outcome {
    let mut last = GRANTED;
    for _ in 0..n {
        last = outcome(lockout.reserve(subject).await.unwrap());
    }
    last
}

pub async fn granted(lockout: &AccountLockout, subject: &Subject) -> Attempt {
    match lockout.reserve(subject).await.unwrap() {
        Reserved::Granted(attempt) => attempt,
        Reserved::Locked { retry_after } => panic!("unexpectedly locked for {retry_after:?}"),
    }
}

/// A completed login of `name` from `addr`.
pub async fn succeed(lockout: &AccountLockout, name: &str, addr: &str) {
    let attempt = granted(lockout, &subject(name, addr)).await;
    lockout.release(attempt).await.unwrap();
}

pub async fn row(db: &TestDb, key: &str) -> Option<auth_lockout::Model> {
    auth_lockout::Entity::find_by_id(key)
        .one(&db.conn)
        .await
        .unwrap()
}

pub async fn failures(db: &TestDb, key: &str) -> i32 {
    row(db, key).await.map_or(0, |r| r.failures)
}

pub async fn row_count(db: &TestDb) -> u64 {
    auth_lockout::Entity::find().count(&db.conn).await.unwrap()
}

/// `locked_until` (unix ms) of `name`'s global row, if set.
pub async fn global_locked_until(db: &TestDb, name: &str) -> Option<i64> {
    let key = subject(name, IP_A).global_key();
    row(db, &key)
        .await
        .and_then(|r| r.locked_until)
        .map(|t| t.timestamp_millis())
}
