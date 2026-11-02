//! Persistent account lockout (M2-06): window counting, exponential backoff with cap and
//! decay, reset on success, anti-enumeration, exact counting under concurrency, restart
//! persistence and cleanup. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::{MINUTE, T0_MS, clock, set, settings};
use deskhaven_core::auth::limiter::{
    AccountLockout, LEVEL_DECAY, LOCKOUT_RETENTION, LockStatus, LockoutKey, MAX_LOCK,
};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::settings::SettingsService;
use deskhaven_db::entity::auth_lockout;
use deskhaven_db::entity::user::{self, AuthSource, Role, Status};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait};
use serde_json::json;
use tokio::task::JoinSet;

const PARALLEL: usize = 20;

struct Fixture {
    settings: SettingsService,
    clock: Arc<ManualClock>,
    lockout: AccountLockout,
}

/// `max_failures`, window 15m, base duration 15m.
async fn fixture(db: &TestDb, max_failures: u32) -> Fixture {
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

fn key(name: &str) -> LockoutKey {
    LockoutKey::username(name)
}

fn locked(minutes: u64) -> LockStatus {
    LockStatus::Locked {
        retry_after: MINUTE * u32::try_from(minutes).unwrap(),
    }
}

async fn fail(lockout: &AccountLockout, name: &str, n: usize) -> LockStatus {
    let mut status = LockStatus::Allowed;
    for _ in 0..n {
        status = lockout.record_failure(&key(name)).await.unwrap();
    }
    status
}

async fn row(db: &TestDb, name: &str) -> Option<auth_lockout::Model> {
    auth_lockout::Entity::find_by_id(key(name).as_str())
        .one(&db.conn)
        .await
        .unwrap()
}

async fn insert_user(db: &TestDb, username: &str) {
    let t0 = time::from_millis(T0_MS).unwrap();
    user::ActiveModel {
        id: NotSet,
        username: Set(username.to_owned()),
        username_norm: Set(username.to_ascii_lowercase()),
        display_name: Set(username.to_owned()),
        email: Set(None),
        email_norm: Set(None),
        email_verified: Set(false),
        password_hash: Set(Some(
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".to_owned(),
        )),
        auth_source: Set(AuthSource::Local),
        role: Set(Role::User),
        status: Set(Status::Active),
        must_change_password: Set(false),
        totp_secret_enc: Set(None),
        totp_enabled_at: Set(None),
        totp_last_step: Set(None),
        note: Set(String::new()),
        avatar_path: Set(None),
        last_login_at: Set(None),
        password_changed_at: Set(None),
        created_at: Set(t0),
        updated_at: Set(t0),
    }
    .insert(&db.conn)
    .await
    .unwrap();
}

#[tokio::test]
async fn locks_after_max_failures_within_the_window() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;

        assert_eq!(fail(&f.lockout, "alice", 2).await, LockStatus::Allowed);
        assert_eq!(
            f.lockout.check(&key("alice")).await.unwrap(),
            LockStatus::Allowed
        );
        f.clock.advance(MINUTE);
        assert_eq!(fail(&f.lockout, "alice", 1).await, locked(15));

        assert_eq!(f.lockout.check(&key("alice")).await.unwrap(), locked(15));
        f.clock.advance(MINUTE * 14);
        assert_eq!(f.lockout.check(&key("alice")).await.unwrap(), locked(1));
        f.clock.advance(MINUTE);
        assert_eq!(
            f.lockout.check(&key("alice")).await.unwrap(),
            LockStatus::Allowed
        );
    })
    .await;
}

#[tokio::test]
async fn failures_spread_across_windows_do_not_lock() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;

        for _ in 0..5 {
            assert_eq!(fail(&f.lockout, "alice", 2).await, LockStatus::Allowed);
            f.clock.advance(MINUTE * 15);
        }

        assert_eq!(
            f.lockout.check(&key("alice")).await.unwrap(),
            LockStatus::Allowed
        );
        assert_eq!(row(db, "alice").await.unwrap().failures, 2);
    })
    .await;
}

#[tokio::test]
async fn window_and_threshold_follow_settings_changes() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        assert_eq!(fail(&f.lockout, "alice", 2).await, LockStatus::Allowed);

        set(&f.settings, "auth.lockout.max_failures", json!(3)).await;
        set(&f.settings, "auth.lockout.duration", json!("2m")).await;
        assert_eq!(fail(&f.lockout, "alice", 1).await, locked(2));

        f.clock.advance(MINUTE * 2);
        set(&f.settings, "auth.lockout.window", json!("1m")).await;
        assert_eq!(fail(&f.lockout, "bob", 2).await, LockStatus::Allowed);
        f.clock.advance(MINUTE);
        assert_eq!(fail(&f.lockout, "bob", 2).await, LockStatus::Allowed);
    })
    .await;
}

#[tokio::test]
async fn repeated_lockouts_back_off_exponentially_up_to_the_cap() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        let mut minutes = Vec::new();

        for _ in 0..9 {
            let LockStatus::Locked { retry_after } = fail(&f.lockout, "alice", 2).await else {
                panic!("{}: expected a lock", db.backend);
            };
            minutes.push(retry_after.as_secs() / 60);
            f.clock.advance(retry_after);
        }

        assert_eq!(minutes, [15, 30, 60, 120, 240, 480, 960, 1440, 1440]);
        assert_eq!(MAX_LOCK, MINUTE * 1440);
        assert_eq!(row(db, "alice").await.unwrap().lock_level, 8);
    })
    .await;
}

#[tokio::test]
async fn backoff_level_resets_after_a_quiet_period() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        assert_eq!(fail(&f.lockout, "alice", 2).await, locked(15));
        f.clock.advance(MINUTE * 15);
        assert_eq!(fail(&f.lockout, "alice", 2).await, locked(30));

        f.clock.advance(MINUTE * 30 + LEVEL_DECAY);

        assert_eq!(fail(&f.lockout, "alice", 2).await, locked(15));
    })
    .await;
}

#[tokio::test]
async fn success_resets_failures_and_backoff() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        fail(&f.lockout, "alice", 2).await;
        f.lockout.record_success(&key("alice")).await.unwrap();
        assert!(row(db, "alice").await.is_none());
        assert_eq!(fail(&f.lockout, "alice", 2).await, LockStatus::Allowed);

        assert_eq!(fail(&f.lockout, "alice", 1).await, locked(15));
        f.clock.advance(MINUTE * 15);
        f.lockout.record_success(&key("alice")).await.unwrap();

        assert_eq!(fail(&f.lockout, "alice", 3).await, locked(15));
    })
    .await;
}

#[tokio::test]
async fn unknown_usernames_lock_exactly_like_existing_ones() {
    each_migrated_backend(async |db: &TestDb| {
        insert_user(db, "alice").await;
        let f = fixture(db, 3).await;

        let mut existing = Vec::new();
        let mut unknown = Vec::new();
        for _ in 0..4 {
            existing.push(f.lockout.record_failure(&key("alice")).await.unwrap());
            unknown.push(f.lockout.record_failure(&key("mallory")).await.unwrap());
        }
        existing.push(f.lockout.check(&key("alice")).await.unwrap());
        unknown.push(f.lockout.check(&key("mallory")).await.unwrap());

        assert_eq!(existing, unknown);
        assert_eq!(existing[2], locked(15));
        let (a, m) = (
            row(db, "alice").await.unwrap(),
            row(db, "mallory").await.unwrap(),
        );
        assert_eq!(
            (a.failures, a.locked_until, a.lock_level),
            (m.failures, m.locked_until, m.lock_level)
        );
    })
    .await;
}

#[tokio::test]
async fn usernames_are_counted_case_insensitively_and_malformed_ones_together() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        fail(&f.lockout, "Alice", 1).await;
        fail(&f.lockout, "ALICE", 1).await;
        assert_eq!(fail(&f.lockout, "alice", 1).await, locked(15));

        // NUL is rejected by PostgreSQL text; malformed names never reach the table verbatim.
        fail(&f.lockout, "nul\0byte", 1).await;
        fail(&f.lockout, "white space", 1).await;
        assert_eq!(fail(&f.lockout, &"x".repeat(1000), 1).await, locked(15));
        let rows = auth_lockout::Entity::find().count(&db.conn).await.unwrap();
        assert_eq!(rows, 2, "{}", db.backend);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_failures_are_all_counted() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 100).await;
        let mut tasks = JoinSet::new();
        for _ in 0..PARALLEL {
            let lockout = f.lockout.clone();
            tasks.spawn(async move { lockout.record_failure(&key("alice")).await });
        }

        let results = tasks.join_all().await;

        assert!(
            results
                .iter()
                .all(|r| *r.as_ref().unwrap() == LockStatus::Allowed)
        );
        let row = row(db, "alice").await.unwrap();
        assert_eq!(
            row.failures,
            i32::try_from(PARALLEL).unwrap(),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_failures_past_the_threshold_lock_once() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        let mut tasks = JoinSet::new();
        for _ in 0..PARALLEL {
            let lockout = f.lockout.clone();
            tasks.spawn(async move { lockout.record_failure(&key("alice")).await });
        }

        let results: Vec<_> = tasks
            .join_all()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();

        let allowed = results
            .iter()
            .filter(|s| **s == LockStatus::Allowed)
            .count();
        assert_eq!(allowed, 4, "{}: {results:?}", db.backend);
        let row = row(db, "alice").await.unwrap();
        assert_eq!((row.failures, row.lock_level), (0, 0), "{}", db.backend);
        assert_eq!(f.lockout.check(&key("alice")).await.unwrap(), locked(15));
    })
    .await;
}

#[tokio::test]
async fn lock_survives_a_new_instance() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        fail(&f.lockout, "alice", 2).await;
        fail(&f.lockout, "bob", 3).await;

        let restarted_settings = settings(db).await;
        let restarted = AccountLockout::new(db.conn.clone(), restarted_settings, f.clock.clone());

        assert_eq!(restarted.check(&key("bob")).await.unwrap(), locked(15));
        assert_eq!(fail(&restarted, "alice", 1).await, locked(15));
    })
    .await;
}

#[tokio::test]
async fn cleanup_removes_only_rows_past_retention() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        fail(&f.lockout, "stale", 1).await;
        fail(&f.lockout, "locked", 2).await;
        f.clock.advance(LOCKOUT_RETENTION - MINUTE);
        fail(&f.lockout, "fresh", 1).await;
        assert_eq!(f.lockout.cleanup().await.unwrap(), 0);

        f.clock.advance(MINUTE);
        assert_eq!(f.lockout.cleanup().await.unwrap(), 2);

        assert!(row(db, "stale").await.is_none());
        assert!(row(db, "locked").await.is_none());
        assert!(row(db, "fresh").await.is_some());
        assert_eq!(
            f.lockout.check(&key("fresh")).await.unwrap(),
            LockStatus::Allowed
        );
    })
    .await;
}

#[tokio::test]
async fn cleanup_keeps_a_level_that_still_matters() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        assert_eq!(fail(&f.lockout, "alice", 2).await, locked(15));
        f.clock.advance(MINUTE * 15 + LEVEL_DECAY - MINUTE);

        assert_eq!(f.lockout.cleanup().await.unwrap(), 0);

        assert_eq!(fail(&f.lockout, "alice", 2).await, locked(30));
    })
    .await;
}
