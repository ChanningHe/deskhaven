//! The login handler facade (M2-06): IP bucket first, then the attempt reservation; the same
//! `Throttled` for every reason and for existing and unknown accounts alike.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use common::{MINUTE, clock, lockout, set, settings};
use deskhaven_core::auth::limiter::{
    Attempt, ClientAddr, LOCKOUT_RETENTION, LockStatus, LoginThrottle, PendingSubject,
    ThrottleError,
};
use deskhaven_core::clock::ManualClock;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

fn client(addr: &str) -> ClientAddr {
    ClientAddr {
        addr: addr.parse::<IpAddr>().unwrap(),
        is_proxy_fallback: false,
    }
}

fn proxy(addr: &str) -> ClientAddr {
    ClientAddr {
        is_proxy_fallback: true,
        ..client(addr)
    }
}

fn retry_after(result: Result<Attempt, ThrottleError>) -> Duration {
    match result {
        Err(ThrottleError::Throttled { retry_after }) => retry_after,
        other => panic!("expected Throttled, got {other:?}"),
    }
}

/// max_failures 3, 4 requests per minute per IP.
async fn throttle(db: &TestDb) -> (LoginThrottle, Arc<ManualClock>) {
    let settings = settings(db).await;
    set(&settings, "auth.lockout.max_failures", json!(3)).await;
    set(&settings, "auth.ip_rate_limit.per_minute", json!(4)).await;
    let clock = clock();
    let throttle = LoginThrottle::new(db.conn.clone(), settings, clock.clone());
    (throttle, clock)
}

/// One failed login: pre-check, then report the failure.
async fn failed_login(throttle: &LoginThrottle, from: ClientAddr, username: &str) -> LockStatus {
    let attempt = throttle.pre_check(from, username).await.unwrap();
    throttle.on_failure(attempt)
}

#[tokio::test]
async fn ip_bucket_is_checked_before_any_attempt_is_counted() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;
        for name in ["a", "b", "c", "d"] {
            let attempt = throttle.pre_check(client("192.0.2.1"), name).await.unwrap();
            throttle.on_success(attempt).await.unwrap();
        }

        let wait = retry_after(throttle.pre_check(client("192.0.2.1"), "carol").await);

        assert_eq!(wait, Duration::from_secs(15));
        let carol = throttle
            .pre_check(client("192.0.2.2"), "carol")
            .await
            .unwrap();
        assert_eq!(throttle.on_failure(carol), LockStatus::Allowed);
    })
    .await;
}

#[tokio::test]
async fn failures_lock_the_username_for_that_source_only() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, clock) = throttle(db).await;
        failed_login(&throttle, client("198.51.100.7"), "alice").await;
        failed_login(&throttle, client("198.51.100.7"), "alice").await;
        let third = failed_login(&throttle, client("198.51.100.7"), "Alice").await;
        assert_eq!(
            third,
            LockStatus::Locked {
                retry_after: MINUTE * 15
            }
        );

        let wait = retry_after(throttle.pre_check(client("198.51.100.7"), "alice").await);
        assert_eq!(wait, MINUTE * 15);
        let elsewhere = throttle
            .pre_check(client("203.0.113.9"), "alice")
            .await
            .unwrap();
        throttle.on_failure(elsewhere);

        clock.advance(MINUTE * 15);
        let again = throttle
            .pre_check(client("198.51.100.7"), "alice")
            .await
            .unwrap();
        throttle.on_success(again).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn proxy_fallback_is_limited_and_counted_under_the_fallback_source() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;
        for i in 0..3 {
            failed_login(&throttle, proxy(&format!("10.0.0.{i}")), "alice").await;
        }

        retry_after(throttle.pre_check(proxy("10.0.0.9"), "alice").await);
        let bob = throttle.pre_check(proxy("10.0.0.9"), "bob").await.unwrap();
        throttle.on_success(bob).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn throttled_is_identical_for_unknown_and_existing_accounts() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;
        let mut outcomes = Vec::new();
        for (name, from) in [("alice", "192.0.2.10"), ("no-such-user", "192.0.2.20")] {
            let mut statuses = Vec::new();
            for _ in 0..3 {
                statuses.push(failed_login(&throttle, client(from), name).await);
            }
            let wait = retry_after(throttle.pre_check(client(from), name).await);
            outcomes.push((statuses, wait));
        }

        assert_eq!(outcomes[0], outcomes[1]);
        assert_eq!(
            outcomes[0].0[2],
            LockStatus::Locked {
                retry_after: MINUTE * 15
            }
        );
    })
    .await;
}

#[tokio::test]
async fn success_clears_the_count_and_refund_never_counts() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, clock) = throttle(db).await;
        let from = client("192.0.2.1");
        failed_login(&throttle, from, "alice").await;
        failed_login(&throttle, from, "alice").await;
        let ok = throttle.pre_check(from, "ALICE").await.unwrap();
        throttle.on_success(ok).await.unwrap();

        clock.advance(MINUTE);
        assert_eq!(
            failed_login(&throttle, from, "alice").await,
            LockStatus::Allowed
        );
        for _ in 0..3 {
            let busy = throttle.pre_check(from, "alice").await.unwrap();
            throttle.refund(busy).await.unwrap();
        }
        clock.advance(MINUTE);
        assert_eq!(
            failed_login(&throttle, from, "alice").await,
            LockStatus::Allowed
        );
    })
    .await;
}

#[tokio::test]
async fn unlock_lifts_every_lock_of_the_username() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, clock) = throttle(db).await;
        for from in ["192.0.2.1", "192.0.2.2"] {
            for _ in 0..3 {
                failed_login(&throttle, client(from), "alice").await;
            }
            retry_after(throttle.pre_check(client(from), "alice").await);
        }

        assert_eq!(throttle.unlock("Alice").await.unwrap(), 3);
        clock.advance(Duration::from_secs(15)); // one IP token back; the locks would last 15m

        let attempt = throttle
            .pre_check(client("192.0.2.1"), "alice")
            .await
            .unwrap();
        throttle.on_success(attempt).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn cleanup_sweeps_ip_buckets_and_stale_rows() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, clock) = throttle(db).await;
        failed_login(&throttle, client("192.0.2.1"), "alice").await;
        assert_eq!(throttle.ip().tracked_clients(), 1);

        clock.advance(LOCKOUT_RETENTION);

        assert_eq!(throttle.cleanup().await.unwrap(), 2);
        assert_eq!(throttle.ip().tracked_clients(), 0);
    })
    .await;
}

// --- M-1: two-step logins share one budget ---------------------------------------------------

#[tokio::test]
async fn password_ok_with_failing_second_factor_reaches_the_lock() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;
        let from = client("192.0.2.1");
        let mut verifications = 0;

        // A password holder loops: password ok -> one TOTP guess -> password again ...
        let throttled = loop {
            let password = match throttle.pre_check(from, "alice").await {
                Ok(attempt) => attempt,
                Err(e) => break e,
            };
            verifications += 1;
            let _pending = throttle.on_password_ok_pending_2fa(password);
            let totp = match throttle.pre_check(from, "alice").await {
                Ok(attempt) => attempt,
                Err(e) => break e,
            };
            verifications += 1;
            throttle.on_failure(totp);
            assert!(verifications < 10, "{}: the budget never ran out", db.backend);
        };

        assert!(
            matches!(throttled, ThrottleError::Throttled { retry_after } if retry_after == MINUTE * 15),
            "{}: {throttled:?}",
            db.backend
        );
        assert_eq!(verifications, 3, "{}: max_failures caps both steps together", db.backend);
    })
    .await;
}

#[tokio::test]
async fn completed_second_factor_releases_and_marks_the_source_known() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;
        let from = client("192.0.2.1");
        let password = throttle.pre_check(from, "alice").await.unwrap();
        let pending = throttle.on_password_ok_pending_2fa(password);
        let totp = throttle.pre_check(from, "alice").await.unwrap();

        throttle.complete(pending, totp).await.unwrap();

        let alice = lockout::subject("alice", "192.0.2.1");
        assert!(lockout::row(db, &alice.global_key()).await.is_none());
        let source = lockout::row(db, &alice.source_key()).await.unwrap();
        assert_eq!(source.failures, 0, "{}", db.backend);
        assert!(source.last_success_at.is_some());
    })
    .await;
}

#[tokio::test]
async fn completing_from_another_source_marks_both_sources_known() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;
        let password = throttle
            .pre_check(client("192.0.2.1"), "alice")
            .await
            .unwrap();
        let pending = throttle.on_password_ok_pending_2fa(password);
        let totp = throttle
            .pre_check(client("198.51.100.9"), "alice")
            .await
            .unwrap();

        throttle.complete(pending, totp).await.unwrap();

        for ip in ["192.0.2.1", "198.51.100.9"] {
            let key = lockout::subject("alice", ip).source_key();
            let row = lockout::row(db, &key).await.unwrap();
            assert_eq!(row.failures, 0, "{}: {ip}", db.backend);
            assert!(row.last_success_at.is_some(), "{}: {ip}", db.backend);
        }
    })
    .await;
}

#[test]
fn pending_subject_round_trips_through_stored_parts_and_rejects_tampering() {
    let subject = lockout::subject("Alice", "2001:db8:1:2::9");
    let pending = PendingSubject::from_parts(subject.name(), subject.source()).unwrap();
    assert_eq!(pending.subject(), &subject);
    assert_eq!(pending.parts(), ("alice", "2001:db8:1::"));

    for (name, source) in [
        ("alice", "192.0.2.1"),
        ("alice", "fallback"),
        ("~malformed", "2001:db8:1::"),
    ] {
        assert!(
            PendingSubject::from_parts(name, source).is_some(),
            "{name} {source}"
        );
    }
    for (name, source) in [
        ("Alice", "192.0.2.1"),        // not normalized
        ("al ice", "192.0.2.1"),       // malformed but not the shared name
        ("alice", "*"),                // the global row
        ("alice", "2001:db8:1:2::"),   // not a /48 network
        ("alice", "::ffff:192.0.2.1"), // not canonical
        ("alice", "garbage"),
        ("alice|x", "192.0.2.1"),
    ] {
        assert!(
            PendingSubject::from_parts(name, source).is_none(),
            "{name} {source}"
        );
    }
}
