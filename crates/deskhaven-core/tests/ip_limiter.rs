//! Per-IP login token bucket (M2-06): quota from settings with hot reload, /64 keying,
//! trusted-proxy exemption, bounded memory. Time is a manual clock.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::IpAddr;
use std::time::Duration;

use common::{MINUTE, clock, set, settings};
use deskhaven_core::auth::limiter::{ClientAddr, IpLimiter, SWEEP_EVERY};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

fn client(addr: &str) -> ClientAddr {
    ClientAddr {
        addr: addr.parse::<IpAddr>().unwrap(),
        is_proxy_fallback: false,
    }
}

fn allowed(limiter: &IpLimiter, addr: &str, n: usize) -> usize {
    (0..n)
        .filter(|_| limiter.check(client(addr)).is_ok())
        .count()
}

#[tokio::test]
async fn allows_per_minute_then_throttles_until_a_token_refills() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(3)).await;
        let clock = clock();
        let limiter = IpLimiter::new(settings, clock.clone());

        assert_eq!(allowed(&limiter, "192.0.2.1", 3), 3);
        let retry_after = limiter.check(client("192.0.2.1")).unwrap_err();
        assert_eq!(retry_after, Duration::from_secs(20));

        clock.advance(Duration::from_secs(19));
        assert!(limiter.check(client("192.0.2.1")).is_err());
        clock.advance(Duration::from_secs(1));
        assert!(limiter.check(client("192.0.2.1")).is_ok());
        assert!(limiter.check(client("192.0.2.1")).is_err());
    })
    .await;
}

#[tokio::test]
async fn default_quota_is_twenty_per_minute() {
    each_migrated_backend(async |db: &TestDb| {
        let limiter = IpLimiter::new(settings(db).await, clock());

        assert_eq!(allowed(&limiter, "192.0.2.1", 25), 20);
    })
    .await;
}

#[tokio::test]
async fn one_ipv6_slash64_shares_a_bucket_and_others_are_independent() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(2)).await;
        let limiter = IpLimiter::new(settings, clock());

        assert!(limiter.check(client("2001:db8:1:1::1")).is_ok());
        assert!(
            limiter
                .check(client("2001:db8:1:1:ffff:ffff:ffff:ffff"))
                .is_ok()
        );
        assert!(limiter.check(client("2001:db8:1:1::2")).is_err());

        assert_eq!(allowed(&limiter, "2001:db8:1:2::1", 3), 2);
        assert_eq!(allowed(&limiter, "192.0.2.1", 3), 2);
        assert_eq!(allowed(&limiter, "192.0.2.2", 3), 2);
    })
    .await;
}

#[tokio::test]
async fn ipv4_mapped_ipv6_shares_the_ipv4_bucket() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(2)).await;
        let limiter = IpLimiter::new(settings, clock());

        assert!(limiter.check(client("192.0.2.9")).is_ok());
        assert!(limiter.check(client("::ffff:192.0.2.9")).is_ok());
        assert!(limiter.check(client("192.0.2.9")).is_err());
    })
    .await;
}

#[tokio::test]
async fn proxy_fallback_addresses_are_never_limited() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(1)).await;
        let limiter = IpLimiter::new(settings, clock());
        let proxy = ClientAddr {
            is_proxy_fallback: true,
            ..client("10.0.0.1")
        };

        let ok = (0..100).filter(|_| limiter.check(proxy).is_ok()).count();

        assert_eq!(ok, 100);
        assert_eq!(limiter.tracked_clients(), 0);
        // The same address seen as a real client is limited.
        assert_eq!(allowed(&limiter, "10.0.0.1", 2), 1);
    })
    .await;
}

#[tokio::test]
async fn settings_changes_apply_on_the_next_check() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(2)).await;
        let limiter = IpLimiter::new(settings.clone(), clock());
        assert_eq!(allowed(&limiter, "192.0.2.1", 3), 2);

        set(&settings, "auth.ip_rate_limit.per_minute", json!(5)).await;
        assert_eq!(allowed(&limiter, "192.0.2.1", 10), 5);

        set(&settings, "auth.ip_rate_limit.per_minute", json!(1)).await;
        assert_eq!(allowed(&limiter, "192.0.2.1", 3), 1);
    })
    .await;
}

#[tokio::test]
async fn refilled_buckets_are_swept() {
    each_migrated_backend(async |db: &TestDb| {
        let clock = clock();
        let limiter = IpLimiter::new(settings(db).await, clock.clone());
        for i in 0..100 {
            limiter
                .check(client(&format!("2001:db8:{i:x}::1")))
                .unwrap();
        }
        assert_eq!(limiter.tracked_clients(), 100);

        limiter.sweep();
        assert_eq!(limiter.tracked_clients(), 100, "recent buckets must stay");

        clock.advance(MINUTE);
        limiter.sweep();
        assert_eq!(limiter.tracked_clients(), 0);
    })
    .await;
}

#[tokio::test]
async fn checks_sweep_periodically_without_an_explicit_call() {
    each_migrated_backend(async |db: &TestDb| {
        let clock = clock();
        let limiter = IpLimiter::new(settings(db).await, clock.clone());
        for i in 0..100 {
            limiter.check(client(&format!("198.51.100.{i}"))).unwrap();
        }
        clock.advance(MINUTE);

        // The SWEEP_EVERY-th check overall triggers the sweep.
        for _ in 100..SWEEP_EVERY {
            let _ = limiter.check(client("203.0.113.1"));
        }

        assert_eq!(limiter.tracked_clients(), 1);
    })
    .await;
}
