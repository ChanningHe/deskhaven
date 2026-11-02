//! Lockout sources (M2-06 round 2): IPv6 /48 source rows, known-good sources that bypass
//! the global row, the single-source guarantee, the hard row cap, lock-free cleanup and
//! exact counts under concurrent refunds. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::lockout::*;
use common::{MINUTE, T0_MS, set};
use deskhaven_core::auth::limiter::{KNOWN_SOURCE_RETENTION, LOCKOUT_RETENTION, MAX_LOCKOUT_ROWS};
use deskhaven_core::clock::Clock;
use deskhaven_db::entity::auth_lockout;
use deskhaven_db::testing::{Backend, TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, QuerySelect};
use serde_json::json;
use tokio::task::JoinSet;

/// A distinct IPv6 /48 per `i`.
fn slash48(i: u32) -> String {
    format!("2001:db8:{i:x}::1")
}

// --- H-1a: one source row per IPv6 /48 -----------------------------------------------------

#[tokio::test]
async fn four_slash64s_in_one_slash48_share_one_source_row() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        let slash64s = [
            "2001:db8:7:1::1",
            "2001:db8:7:2::1",
            "2001:db8:7:3::1",
            "2001:db8:7:4::1",
        ];

        let mut outcomes = Vec::new();
        for _ in 0..2 {
            for addr in slash64s {
                outcomes.push(fail(&f.lockout, &subject("alice", addr), 1).await);
            }
        }

        assert_eq!(
            subject("alice", slash64s[3]).source_key(),
            "user:alice|2001:db8:7::"
        );
        let granted = outcomes
            .iter()
            .filter(|o| matches!(o, Outcome::Granted(_)))
            .count();
        assert_eq!(granted, 5, "{}: {outcomes:?}", db.backend);
        assert_eq!(outcomes[4], starts_lock(15));
        assert_eq!(
            global_locked_until(db, "alice").await,
            None,
            "{}",
            db.backend
        );
        assert_eq!(failures(db, &subject("alice", IP_A).global_key()).await, 5);
    })
    .await;
}

// --- H-1b: known-good sources --------------------------------------------------------------

#[tokio::test]
async fn rotating_slash48s_lock_fresh_sources_but_not_a_known_source() {
    each_migrated_backend(async |db: &TestDb| {
        // Defaults: 5 per source, 20 globally, 15m.
        let f = fixture(db, 5).await;
        succeed(&f.lockout, "admin", IP_A).await;
        let mut next = 0;

        // 24h of the reviewer's attack: every 15m, four fresh /48s × 5 trip the global row.
        for round in 0..96 {
            for _ in 0..4 {
                next += 1;
                fail(&f.lockout, &subject("admin", &slash48(next)), 5).await;
            }
            let until = global_locked_until(db, "admin").await.unwrap_or_default();
            assert!(until > f.clock.now_ms(), "{}: round {round}", db.backend);
            let fresh = fail(&f.lockout, &subject("admin", &slash48(0xfff0)), 1).await;
            assert!(
                matches!(fresh, Outcome::Throttled(_)),
                "{}: round {round}",
                db.backend
            );

            let known = granted(&f.lockout, &subject("admin", IP_A)).await;
            drop(known); // a failed attempt from the known source: still allowed next round
            f.clock.advance(MINUTE * 15);
        }
    })
    .await;
}

#[tokio::test]
async fn success_deletes_the_global_row_and_marks_the_source_known() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        fail(&f.lockout, &subject("alice", IP_A), 2).await;

        succeed(&f.lockout, "alice", IP_A).await;

        let source = row(db, &subject("alice", IP_A).source_key()).await.unwrap();
        assert_eq!(
            (source.failures, source.locked_until, source.lock_level),
            (0, None, 0),
            "{}",
            db.backend
        );
        let at = source.last_success_at.map(|t| t.timestamp_millis());
        assert_eq!(at, Some(T0_MS));
        assert!(
            row(db, &subject("alice", IP_A).global_key())
                .await
                .is_none()
        );
    })
    .await;
}

#[tokio::test]
async fn known_source_past_retention_is_blocked_by_the_global_row() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        succeed(&f.lockout, "admin", IP_A).await;
        f.clock.advance(KNOWN_SOURCE_RETENTION);
        for i in 1..=4 {
            fail(&f.lockout, &subject("admin", &slash48(i)), 5).await;
        }
        assert!(global_locked_until(db, "admin").await.is_some());

        assert_eq!(
            fail(&f.lockout, &subject("admin", IP_A), 1).await,
            throttled(15)
        );
    })
    .await;
}

#[tokio::test]
async fn known_source_just_inside_retention_still_bypasses_the_global_row() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        succeed(&f.lockout, "admin", IP_A).await;
        f.clock.advance(KNOWN_SOURCE_RETENTION - MINUTE);
        for i in 1..=4 {
            fail(&f.lockout, &subject("admin", &slash48(i)), 5).await;
        }

        assert_eq!(fail(&f.lockout, &subject("admin", IP_A), 1).await, GRANTED);
    })
    .await;
}

#[tokio::test]
async fn known_source_is_still_bound_by_its_own_row() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        succeed(&f.lockout, "alice", IP_A).await;

        assert_eq!(
            fail(&f.lockout, &subject("alice", IP_A), 3).await,
            starts_lock(15)
        );
        assert_eq!(
            fail(&f.lockout, &subject("alice", IP_A), 1).await,
            throttled(15)
        );
    })
    .await;
}

#[tokio::test]
async fn zero_failure_sources_are_not_exempt() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;
        for i in 1..=4 {
            fail(&f.lockout, &subject("admin", &slash48(i)), 5).await;
        }

        // A source never seen before (zero failures) is not a known source.
        assert_eq!(
            fail(&f.lockout, &subject("admin", IP_B), 1).await,
            throttled(15)
        );
    })
    .await;
}

/// Attempts from one source whenever it is allowed, for `hours`; returns how many times the
/// global row was found locked.
async fn single_source_global_locks(f: &Fixture, db: &TestDb, hours: i64) -> usize {
    let end = f.clock.now_ms() + hours * 3_600_000;
    let alice = subject("alice", IP_A);
    let mut global_locks = 0;
    while f.clock.now_ms() < end {
        match outcome(f.lockout.reserve(&alice).await.unwrap()) {
            Outcome::Throttled(wait) => f.clock.advance(wait),
            Outcome::Granted(_) => f.clock.advance(Duration::from_secs(1)),
        }
        if global_locked_until(db, "alice").await.is_some() {
            global_locks += 1;
        }
    }
    global_locks
}

#[tokio::test]
async fn a_single_source_never_trips_the_global_row_with_default_settings() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 5).await;

        assert_eq!(
            single_source_global_locks(&f, db, 48).await,
            0,
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_single_source_never_trips_the_global_row_at_the_widest_allowed_window() {
    each_migrated_backend(async |db: &TestDb| {
        // window == 4 × duration, the most the cross-key rule allows; thresholds 1 and 4.
        let f = fixture(db, 1).await;
        set(&f.settings, "auth.lockout.window", json!("4m")).await;
        set(&f.settings, "auth.lockout.duration", json!("1m")).await;

        assert_eq!(
            single_source_global_locks(&f, db, 48).await,
            0,
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn cleanup_keeps_known_sources_within_retention() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        succeed(&f.lockout, "alice", IP_A).await;
        let key = subject("alice", IP_A).source_key();

        f.clock.advance(KNOWN_SOURCE_RETENTION - MINUTE);
        assert_eq!(f.lockout.cleanup().await.unwrap(), 0, "{}", db.backend);
        assert!(row(db, &key).await.is_some());

        f.clock.advance(MINUTE);
        assert_eq!(f.lockout.cleanup().await.unwrap(), 1);
        assert!(row(db, &key).await.is_none());
    })
    .await;
}

// --- M-2: hard row cap -----------------------------------------------------------------------

#[tokio::test]
async fn cleanup_caps_rows_evicting_the_oldest_unlocked_unknown_rows_first() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 3).await;
        let lockout = f.lockout.clone().with_max_rows(4);
        succeed(&lockout, "alice", IP_A).await; // known source row
        f.clock.advance(MINUTE);
        fail(&lockout, &subject("bob", IP_A), 3).await; // locked source + unlocked global
        for name in ["carol", "dave", "erin"] {
            f.clock.advance(MINUTE);
            fail(&lockout, &subject(name, IP_A), 1).await; // unlocked source + global
        }
        assert_eq!(row_count(db).await, 9);

        // Evictable, oldest first: bob's global, carol's two, dave's two.
        assert_eq!(lockout.cleanup().await.unwrap(), 5, "{}", db.backend);

        assert_eq!(row_count(db).await, 4);
        assert!(
            row(db, &subject("alice", IP_A).source_key())
                .await
                .is_some()
        );
        assert!(row(db, &subject("bob", IP_A).source_key()).await.is_some());
        assert!(row(db, &subject("erin", IP_A).source_key()).await.is_some());
        assert!(row(db, &subject("erin", IP_A).global_key()).await.is_some());
        assert_eq!(MAX_LOCKOUT_ROWS, 100_000);
    })
    .await;
}

#[tokio::test]
async fn cleanup_cap_never_evicts_locked_or_known_rows() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 1).await;
        let lockout = f.lockout.clone().with_max_rows(1);
        succeed(&lockout, "alice", IP_A).await;
        fail(&lockout, &subject("bob", IP_A), 1).await; // locked source + unlocked global

        assert_eq!(lockout.cleanup().await.unwrap(), 1, "{}", db.backend);

        assert_eq!(
            row_count(db).await,
            2,
            "over the cap, but nothing else is evictable"
        );
    })
    .await;
}

// --- L-b: cleanup never waits for row locks ------------------------------------------------

#[tokio::test]
async fn cleanup_skips_rows_locked_by_a_concurrent_transaction() {
    each_migrated_backend(async |db: &TestDb| {
        if db.backend != Backend::Postgres {
            return; // SQLite serializes writers: there are no row locks to skip.
        }
        let f = fixture(db, 3).await;
        fail(&f.lockout, &subject("held", IP_A), 1).await;
        fail(&f.lockout, &subject("free", IP_A), 1).await;
        f.clock.advance(LOCKOUT_RETENTION);
        let held = subject("held", IP_A).global_key();
        let txn = deskhaven_db::begin_write(&db.conn).await.unwrap();
        auth_lockout::Entity::find_by_id(held.as_str())
            .lock_exclusive()
            .one(&txn)
            .await
            .unwrap();

        let cleaned = tokio::time::timeout(Duration::from_secs(5), f.lockout.cleanup()).await;

        assert_eq!(cleaned.expect("cleanup must not wait").unwrap(), 3);
        txn.rollback().await.unwrap();
        assert!(row(db, &held).await.is_some());
    })
    .await;
}

// --- L-d: exact counts under concurrency ---------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_reserves_and_refunds_leave_exact_counts() {
    each_migrated_backend(async |db: &TestDb| {
        let f = fixture(db, 100).await;
        let mut tasks = JoinSet::new();
        for i in 0..PARALLEL {
            let lockout = f.lockout.clone();
            tasks.spawn(async move {
                let mut kept = 0;
                for _ in 0..3 {
                    let attempt = granted(&lockout, &subject("alice", IP_A)).await;
                    if i % 2 == 0 {
                        lockout.refund(attempt).await.unwrap();
                    } else {
                        kept += 1;
                    }
                }
                kept
            });
        }

        let kept: i32 = tasks.join_all().await.into_iter().sum();

        let alice = subject("alice", IP_A);
        assert_eq!(kept, 30);
        assert_eq!(
            failures(db, &alice.source_key()).await,
            kept,
            "{}",
            db.backend
        );
        assert_eq!(
            failures(db, &alice.global_key()).await,
            kept,
            "{}",
            db.backend
        );
    })
    .await;
}
