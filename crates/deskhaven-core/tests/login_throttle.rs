//! The login handler facade (M2-06): IP bucket first, then the account lockout; the same
//! `Throttled` for every reason and for existing and unknown accounts alike.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::IpAddr;
use std::time::Duration;

use common::{MINUTE, clock, set, settings};
use deskhaven_core::auth::limiter::{ClientAddr, LockStatus, LoginThrottle, ThrottleError};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

fn client(addr: &str) -> ClientAddr {
    ClientAddr {
        addr: addr.parse::<IpAddr>().unwrap(),
        is_proxy_fallback: false,
    }
}

fn retry_after(result: Result<(), ThrottleError>) -> Duration {
    match result {
        Err(ThrottleError::Throttled { retry_after }) => retry_after,
        other => panic!("expected Throttled, got {other:?}"),
    }
}

/// max_failures 3, 2 requests per minute per IP.
async fn throttle(
    db: &TestDb,
) -> (
    LoginThrottle,
    std::sync::Arc<deskhaven_core::clock::ManualClock>,
) {
    let settings = settings(db).await;
    set(&settings, "auth.lockout.max_failures", json!(3)).await;
    set(&settings, "auth.ip_rate_limit.per_minute", json!(2)).await;
    let clock = clock();
    (
        LoginThrottle::new(db.conn.clone(), settings, clock.clone()),
        clock,
    )
}

#[tokio::test]
async fn ip_bucket_is_checked_before_the_account() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;

        throttle
            .pre_check(client("192.0.2.1"), "alice")
            .await
            .unwrap();
        throttle
            .pre_check(client("192.0.2.1"), "bob")
            .await
            .unwrap();

        let wait = retry_after(throttle.pre_check(client("192.0.2.1"), "carol").await);
        assert_eq!(wait, Duration::from_secs(30));
        throttle
            .pre_check(client("192.0.2.2"), "carol")
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn locked_accounts_are_throttled_from_any_address() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, clock) = throttle(db).await;
        for _ in 0..3 {
            throttle.on_failure("alice").await.unwrap();
        }

        let wait = retry_after(throttle.pre_check(client("198.51.100.7"), "alice").await);
        assert_eq!(wait, MINUTE * 15);
        let proxy = ClientAddr {
            is_proxy_fallback: true,
            ..client("10.0.0.1")
        };
        retry_after(throttle.pre_check(proxy, "Alice").await);
        throttle.pre_check(proxy, "bob").await.unwrap();

        clock.advance(MINUTE * 15);
        throttle
            .pre_check(client("198.51.100.8"), "alice")
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn throttled_is_identical_for_unknown_and_existing_accounts() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, _) = throttle(db).await;
        let mut outcomes = Vec::new();
        for name in ["alice", "no-such-user"] {
            let mut statuses = Vec::new();
            for _ in 0..3 {
                statuses.push(throttle.on_failure(name).await.unwrap());
            }
            let proxy = ClientAddr {
                is_proxy_fallback: true,
                ..client("10.0.0.1")
            };
            let wait = retry_after(throttle.pre_check(proxy, name).await);
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
async fn success_clears_the_failure_count() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, clock) = throttle(db).await;
        throttle.on_failure("alice").await.unwrap();
        throttle.on_failure("alice").await.unwrap();

        throttle.on_success("ALICE").await.unwrap();

        assert_eq!(
            throttle.on_failure("alice").await.unwrap(),
            LockStatus::Allowed
        );
        assert_eq!(
            throttle.on_failure("alice").await.unwrap(),
            LockStatus::Allowed
        );
        clock.advance(MINUTE);
        throttle
            .pre_check(client("192.0.2.1"), "alice")
            .await
            .unwrap();
    })
    .await;
}

#[tokio::test]
async fn cleanup_sweeps_ip_buckets_and_stale_rows() {
    each_migrated_backend(async |db: &TestDb| {
        let (throttle, clock) = throttle(db).await;
        throttle
            .pre_check(client("192.0.2.1"), "alice")
            .await
            .unwrap();
        throttle.on_failure("alice").await.unwrap();
        assert_eq!(throttle.ip().tracked_clients(), 1);

        clock.advance(deskhaven_core::auth::limiter::LOCKOUT_RETENTION);

        assert_eq!(throttle.cleanup().await.unwrap(), 1);
        assert_eq!(throttle.ip().tracked_clients(), 0);
    })
    .await;
}
