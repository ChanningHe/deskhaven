//! Persistent account lockout (M2-06): count-before-verify reservations, per-source and
//! global rows, exponential backoff with cap and decay, reset on success, refund, unlock,
//! anti-enumeration, exact counting under concurrency, restart persistence and cleanup.
//! Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::lockout::*;
use common::{MINUTE, T0_MS, set, settings};
use deskhaven_core::auth::limiter::{
    AccountLockout, ClientAddr, LEVEL_DECAY, LOCKOUT_RETENTION, LockStatus, MAX_LOCK, Reserved,
    Subject, UNLOCKED_RETENTION,
};
use deskhaven_db::entity::user::{self, AuthSource, Role, Status};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::{NotSet, Set};
use sea_orm::{ActiveModelTrait, EntityTrait};
use serde_json::json;
use tokio::task::JoinSet;

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

// --- counting and locking --------------------------------------------------------------

#[tokio::test]
async fn threshold_attempt_is_granted_and_starts_the_lock() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        let alice = subject("alice", IP_A);

        assert_eq!(fail(&f.lockout, &alice, 2).await, GRANTED);
        assert_eq!(f.lockout.check(&alice).await.unwrap(), LockStatus::Allowed);
        f.clock.advance(MINUTE);
        assert_eq!(fail(&f.lockout, &alice, 1).await, starts_lock(15));
        assert_eq!(fail(&f.lockout, &alice, 1).await, throttled(15));

        f.clock.advance(MINUTE * 14);
        assert_eq!(fail(&f.lockout, &alice, 1).await, throttled(1));
        f.clock.advance(MINUTE);
        assert_eq!(f.lockout.check(&alice).await.unwrap(), LockStatus::Allowed);
        assert_eq!(fail(&f.lockout, &alice, 1).await, GRANTED);
    })
    .await;
}

#[tokio::test]
async fn attempts_spread_across_windows_do_not_lock() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        let alice = subject("alice", IP_A);

        for _ in 0..5 {
            assert_eq!(fail(&f.lockout, &alice, 2).await, GRANTED);
            f.clock.advance(MINUTE * 15);
        }

        assert_eq!(f.lockout.check(&alice).await.unwrap(), LockStatus::Allowed);
        assert_eq!(failures(db, &alice.source_key()).await, 2);
    })
    .await;
}

#[tokio::test]
async fn window_threshold_and_duration_follow_settings_changes() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        let alice = subject("alice", IP_A);
        assert_eq!(fail(&f.lockout, &alice, 2).await, GRANTED);

        set(&f.settings, "auth.lockout.max_failures", json!(3)).await;
        // window <= 4 × duration (cross-key rule): shrink the window first.
        set(&f.settings, "auth.lockout.window", json!("8m")).await;
        set(&f.settings, "auth.lockout.duration", json!("2m")).await;
        assert_eq!(fail(&f.lockout, &alice, 1).await, starts_lock(2));

        let bob = subject("bob", IP_A);
        assert_eq!(fail(&f.lockout, &bob, 2).await, GRANTED);
        f.clock.advance(MINUTE * 8);
        assert_eq!(fail(&f.lockout, &bob, 2).await, GRANTED);
    })
    .await;
}

#[tokio::test]
async fn repeated_lockouts_from_one_source_back_off_exponentially_up_to_the_cap() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        let alice = subject("alice", IP_A);
        let mut minutes = Vec::new();

        for _ in 0..9 {
            let Outcome::Granted(LockStatus::Locked { retry_after }) =
                fail(&f.lockout, &alice, 2).await
            else {
                panic!("{}: expected a lock", db.backend);
            };
            minutes.push(retry_after.as_secs() / 60);
            f.clock.advance(retry_after);
        }

        assert_eq!(minutes, [15, 30, 60, 120, 240, 480, 960, 1440, 1440]);
        assert_eq!(MAX_LOCK, MINUTE * 1440);
        assert_eq!(row(db, &alice.source_key()).await.unwrap().lock_level, 8);
    })
    .await;
}

#[tokio::test]
async fn backoff_level_resets_after_a_quiet_period() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        let alice = subject("alice", IP_A);
        assert_eq!(fail(&f.lockout, &alice, 2).await, starts_lock(15));
        f.clock.advance(MINUTE * 15);
        assert_eq!(fail(&f.lockout, &alice, 2).await, starts_lock(30));

        f.clock.advance(MINUTE * 30 + LEVEL_DECAY);

        assert_eq!(fail(&f.lockout, &alice, 2).await, starts_lock(15));
    })
    .await;
}

// --- H1: per-source and global rows -----------------------------------------------------

#[tokio::test]
async fn one_source_locks_only_its_own_row() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;

        assert_eq!(
            fail(&f.lockout, &subject("alice", IP_A), 3).await,
            starts_lock(15)
        );

        assert_eq!(
            fail(&f.lockout, &subject("alice", IP_A), 1).await,
            throttled(15)
        );
        assert_eq!(fail(&f.lockout, &subject("alice", IP_B), 1).await, GRANTED);
        let same_slash64 = subject("alice", "2001:db8:1:1::1");
        fail(&f.lockout, &same_slash64, 3).await;
        assert_eq!(
            fail(&f.lockout, &subject("alice", "2001:db8:1:1::ffff"), 1).await,
            throttled(15)
        );
    })
    .await;
}

#[tokio::test]
async fn global_row_bounds_distributed_guessing_with_a_fixed_duration() {
    each_migrated_backend(async |db: &TestDb| {
        // Per source 3, globally 12.
        let f = fixture(db, 3).await;
        let from = |i: u32| subject("alice", &format!("203.0.113.{i}"));

        for round in 0..3 {
            let mut last = GRANTED;
            for i in 1..=6 {
                last = fail(&f.lockout, &from(round * 10 + i), 2).await;
            }
            assert_eq!(last, starts_lock(15), "{}: round {round}", db.backend);
            assert_eq!(fail(&f.lockout, &from(99), 1).await, throttled(15));
            let global = row(db, &from(99).global_key()).await.unwrap();
            assert_eq!(global.lock_level, 0, "{}: no backoff globally", db.backend);
            f.clock.advance(MINUTE * 15);
        }
        assert_eq!(fail(&f.lockout, &subject("bob", IP_A), 1).await, GRANTED);
    })
    .await;
}

#[tokio::test]
async fn proxy_fallback_requests_share_the_fallback_source() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        let via = |proxy: &str| {
            Subject::new(
                ClientAddr {
                    addr: proxy.parse().unwrap(),
                    is_proxy_fallback: true,
                },
                "alice",
            )
        };
        assert_eq!(via("10.0.0.1").source_key(), "user:alice|fallback");

        fail(&f.lockout, &via("10.0.0.1"), 1).await;
        assert_eq!(fail(&f.lockout, &via("10.0.0.2"), 1).await, starts_lock(15));

        assert_eq!(fail(&f.lockout, &via("10.0.0.3"), 1).await, throttled(15));
        assert_eq!(
            fail(&f.lockout, &subject("alice", "10.0.0.1"), 1).await,
            GRANTED
        );
    })
    .await;
}

#[tokio::test]
async fn unlock_clears_every_row_of_one_username_only() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        for ip in [IP_A, IP_B, "2001:db8::1"] {
            fail(&f.lockout, &subject("a_b", ip), 2).await;
            fail(&f.lockout, &subject("axb", ip), 2).await;
        }
        assert_eq!(row_count(db).await, 8);

        assert_eq!(f.lockout.unlock("A_B").await.unwrap(), 4);

        assert_eq!(
            row_count(db).await,
            4,
            "{}: `_` must not match `x`",
            db.backend
        );
        assert_eq!(fail(&f.lockout, &subject("a_b", IP_A), 1).await, GRANTED);
        assert_eq!(
            fail(&f.lockout, &subject("axb", IP_A), 1).await,
            throttled(15)
        );
        assert_eq!(f.lockout.unlock("nobody").await.unwrap(), 0);
    })
    .await;
}

// --- M1: count before verify --------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_attempts_grant_at_most_max_failures_per_lock_cycle() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        let mut tasks = JoinSet::new();
        for _ in 0..PARALLEL {
            let lockout = f.lockout.clone();
            tasks.spawn(
                async move { outcome(lockout.reserve(&subject("alice", IP_A)).await.unwrap()) },
            );
        }

        let outcomes = tasks.join_all().await;

        let verifications = outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Granted(_)))
            .count();
        assert_eq!(verifications, 5, "{}: {outcomes:?}", db.backend);
        assert_eq!(
            outcomes.iter().filter(|o| **o == starts_lock(15)).count(),
            1
        );
        let source = row(db, &subject("alice", IP_A).source_key()).await.unwrap();
        assert_eq!(
            (source.failures, source.lock_level),
            (0, 0),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_attempts_from_many_sources_respect_the_global_threshold() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        let mut tasks = JoinSet::new();
        for i in 0..PARALLEL {
            let lockout = f.lockout.clone();
            let s = subject("alice", &format!("203.0.113.{i}"));
            tasks.spawn(async move { outcome(lockout.reserve(&s).await.unwrap()) });
        }

        let outcomes = tasks.join_all().await;

        let verifications = outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Granted(_)))
            .count();
        assert_eq!(verifications, 12, "{}: {outcomes:?}", db.backend);
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_attempts_are_all_counted() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 100).await;
        let mut tasks = JoinSet::new();
        for _ in 0..PARALLEL {
            let lockout = f.lockout.clone();
            tasks.spawn(
                async move { outcome(lockout.reserve(&subject("alice", IP_A)).await.unwrap()) },
            );
        }

        let outcomes = tasks.join_all().await;

        assert!(outcomes.iter().all(|o| *o == GRANTED));
        let alice = subject("alice", IP_A);
        let expected = i32::try_from(PARALLEL).unwrap();
        assert_eq!(
            failures(db, &alice.source_key()).await,
            expected,
            "{}",
            db.backend
        );
        assert_eq!(
            failures(db, &alice.global_key()).await,
            expected,
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn refund_gives_the_attempt_back() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        let alice = subject("alice", IP_A);
        fail(&f.lockout, &alice, 1).await;
        let attempt = granted(&f.lockout, &alice).await;
        assert_eq!(failures(db, &alice.source_key()).await, 2);

        f.lockout.refund(attempt).await.unwrap();

        assert_eq!(failures(db, &alice.source_key()).await, 1);
        assert_eq!(failures(db, &alice.global_key()).await, 1);
    })
    .await;
}

#[tokio::test]
async fn refund_of_the_lock_starting_attempt_lifts_the_lock() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        let alice = subject("alice", IP_A);
        fail(&f.lockout, &alice, 1).await;
        let attempt = granted(&f.lockout, &alice).await;
        assert_eq!(
            attempt.status(),
            LockStatus::Locked {
                retry_after: MINUTE * 15
            }
        );

        f.lockout.refund(attempt).await.unwrap();

        assert_eq!(f.lockout.check(&alice).await.unwrap(), LockStatus::Allowed);
        assert_eq!(failures(db, &alice.source_key()).await, 1);
        assert_eq!(fail(&f.lockout, &alice, 1).await, starts_lock(15));
    })
    .await;
}

#[tokio::test]
async fn refund_after_the_rows_were_cleared_is_a_no_op() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        let attempt = granted(&f.lockout, &subject("alice", IP_A)).await;
        f.lockout.unlock("alice").await.unwrap();

        f.lockout.refund(attempt).await.unwrap();

        assert_eq!(row_count(db).await, 0);
    })
    .await;
}

// --- success, anti-enumeration, persistence -------------------------------------------------

#[tokio::test]
async fn success_deletes_the_global_row_and_keeps_the_source_as_known() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        let alice = subject("alice", IP_A);
        fail(&f.lockout, &subject("alice", IP_B), 2).await;
        fail(&f.lockout, &alice, 1).await;

        let attempt = granted(&f.lockout, &alice).await;
        f.lockout.release(attempt).await.unwrap();

        let source = row(db, &alice.source_key()).await.unwrap();
        assert_eq!(
            (source.failures, source.locked_until, source.lock_level),
            (0, None, 0),
            "{}",
            db.backend
        );
        assert_eq!(
            source.last_success_at.map(|t| t.timestamp_millis()),
            Some(T0_MS)
        );
        assert!(row(db, &alice.global_key()).await.is_none());
        assert_eq!(failures(db, &subject("alice", IP_B).source_key()).await, 2);
        assert_eq!(fail(&f.lockout, &alice, 2).await, GRANTED);
    })
    .await;
}

#[tokio::test]
async fn success_after_a_lock_resets_the_backoff_level() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        let alice = subject("alice", IP_A);
        assert_eq!(fail(&f.lockout, &alice, 2).await, starts_lock(15));
        f.clock.advance(MINUTE * 15);

        let attempt = granted(&f.lockout, &alice).await;
        f.lockout.release(attempt).await.unwrap();

        assert_eq!(fail(&f.lockout, &alice, 2).await, starts_lock(15));
    })
    .await;
}

#[tokio::test]
async fn unknown_usernames_lock_exactly_like_existing_ones() {
    each_migrated_backend(async |db: &TestDb| {
        insert_user(db, "alice").await;
        let f = fixture(db, 3).await;
        let (alice, mallory) = (subject("alice", IP_A), subject("mallory", IP_A));

        let mut existing = Vec::new();
        let mut unknown = Vec::new();
        for _ in 0..5 {
            existing.push(fail(&f.lockout, &alice, 1).await);
            unknown.push(fail(&f.lockout, &mallory, 1).await);
        }

        assert_eq!(existing, unknown);
        assert_eq!(existing[2], starts_lock(15));
        assert_eq!(existing[3], throttled(15));
        for (a, m) in [
            (alice.source_key(), mallory.source_key()),
            (alice.global_key(), mallory.global_key()),
        ] {
            let (a, m) = (row(db, &a).await.unwrap(), row(db, &m).await.unwrap());
            assert_eq!(
                (a.failures, a.locked_until, a.lock_level),
                (m.failures, m.locked_until, m.lock_level)
            );
        }
    })
    .await;
}

#[tokio::test]
async fn usernames_are_counted_case_insensitively_and_malformed_ones_together() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        fail(&f.lockout, &subject("Alice", IP_A), 1).await;
        fail(&f.lockout, &subject("ALICE", IP_A), 1).await;
        assert_eq!(
            fail(&f.lockout, &subject("alice", IP_A), 1).await,
            starts_lock(15)
        );

        // NUL is rejected by PostgreSQL text; malformed names never reach the table verbatim.
        fail(&f.lockout, &subject("nul\0byte", IP_A), 1).await;
        fail(&f.lockout, &subject("white space", IP_A), 1).await;
        let long = "x".repeat(1000);
        assert_eq!(
            fail(&f.lockout, &subject(&long, IP_A), 1).await,
            starts_lock(15)
        );
        assert_eq!(row_count(db).await, 4, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn lock_survives_a_new_instance() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        fail(&f.lockout, &subject("alice", IP_A), 2).await;
        fail(&f.lockout, &subject("bob", IP_A), 3).await;

        let restarted = AccountLockout::new(db.conn.clone(), settings(db).await, f.clock.clone());

        let bob = subject("bob", IP_A);
        assert_eq!(
            restarted.check(&bob).await.unwrap(),
            LockStatus::Locked {
                retry_after: MINUTE * 15
            }
        );
        assert_eq!(
            fail(&restarted, &subject("alice", IP_A), 1).await,
            starts_lock(15)
        );
    })
    .await;
}

// --- L1: success racing attempts ------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn success_refund_and_unlock_racing_attempts_never_error() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 100).await;
        let mut tasks = JoinSet::new();
        for i in 0..PARALLEL {
            let lockout = f.lockout.clone();
            tasks.spawn(async move {
                // Two sources so rows of one username are locked in mixed combinations.
                let alice = subject("alice", if i % 2 == 0 { IP_A } else { IP_B });
                for _ in 0..5 {
                    if i % 5 == 4 {
                        lockout.unlock("alice").await?;
                        continue;
                    }
                    let Reserved::Granted(attempt) = lockout.reserve(&alice).await? else {
                        continue;
                    };
                    match i % 5 {
                        0 | 1 => lockout.release(attempt).await?,
                        2 => lockout.refund(attempt).await?,
                        _ => drop(attempt),
                    }
                }
                Ok::<(), sea_orm::DbErr>(())
            });
        }

        let results = tasks.join_all().await;

        assert!(
            results.iter().all(Result::is_ok),
            "{}: {results:?}",
            db.backend
        );
        // Final state is consistent: counts within 0..threshold, nothing locked (100 >> 25
        // attempts per row), the global row is never above the sum of its sources, and one
        // more sequential success leaves exactly the known source rows.
        let rows = deskhaven_db::entity::auth_lockout::Entity::find()
            .all(&db.conn)
            .await
            .unwrap();
        for r in &rows {
            assert!((0..=100).contains(&r.failures), "{}: {r:?}", db.backend);
            assert_eq!(
                (r.locked_until, r.lock_level),
                (None, 0),
                "{}: {r:?}",
                db.backend
            );
        }
        let alice = |ip| subject("alice", ip);
        let sources = failures(db, &alice(IP_A).source_key()).await
            + failures(db, &alice(IP_B).source_key()).await;
        assert!(failures(db, &alice(IP_A).global_key()).await <= sources);
        succeed(&f.lockout, "alice", IP_A).await;
        assert!(row(db, &alice(IP_A).global_key()).await.is_none());
        assert_eq!(failures(db, &alice(IP_A).source_key()).await, 0);
    })
    .await;
}

// --- cleanup ---------------------------------------------------------------------------------

#[tokio::test]
async fn cleanup_removes_only_rows_past_retention() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        fail(&f.lockout, &subject("locked", IP_A), 2).await;
        f.clock.advance(LOCKOUT_RETENTION - MINUTE);
        fail(&f.lockout, &subject("fresh", IP_A), 1).await;
        // `locked`'s global row never locked: gone after a day.
        assert_eq!(f.lockout.cleanup().await.unwrap(), 1, "{}", db.backend);
        assert!(
            row(db, &subject("locked", IP_A).source_key())
                .await
                .is_some()
        );

        f.clock.advance(MINUTE);
        // `locked`'s source row: past the full retention.
        assert_eq!(f.lockout.cleanup().await.unwrap(), 1);

        assert!(
            row(db, &subject("locked", IP_A).source_key())
                .await
                .is_none()
        );
        assert!(
            row(db, &subject("fresh", IP_A).source_key())
                .await
                .is_some()
        );
    })
    .await;
}

#[tokio::test]
async fn cleanup_removes_never_locked_rows_after_a_day() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        fail(&f.lockout, &subject("idle", IP_A), 1).await;
        fail(&f.lockout, &subject("locked", IP_B), 2).await;
        f.clock.advance(UNLOCKED_RETENTION - MINUTE);
        assert_eq!(f.lockout.cleanup().await.unwrap(), 0);

        f.clock.advance(MINUTE);
        // `idle` source + global, `locked` global; `locked`'s source row keeps its level.
        assert_eq!(f.lockout.cleanup().await.unwrap(), 3, "{}", db.backend);
        assert!(
            row(db, &subject("locked", IP_B).source_key())
                .await
                .is_some()
        );
    })
    .await;
}

#[tokio::test]
async fn cleanup_keeps_a_level_that_still_matters() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 2).await;
        let alice = subject("alice", IP_A);
        assert_eq!(fail(&f.lockout, &alice, 2).await, starts_lock(15));
        f.clock.advance(MINUTE * 15 + LEVEL_DECAY - MINUTE);

        f.lockout.cleanup().await.unwrap();

        assert_eq!(fail(&f.lockout, &alice, 2).await, starts_lock(30));
    })
    .await;
}
