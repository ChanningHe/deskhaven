//! Per-IP login token bucket (M2-06): quota from settings with hot reload, /64 keying,
//! trusted-proxy fallback bucket, bounded memory. Time is a manual clock.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use common::{MINUTE, T0_MS, clock, set, settings};
use deskhaven_core::auth::limiter::{ClientAddr, FALLBACK_FACTOR, IpLimiter, SWEEP_EVERY};
use deskhaven_core::clock::Clock;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time::{self, TimestampMs};
use serde_json::json;

/// A clock whose wall and monotonic readings move independently, like a host whose wall
/// clock is stepped by NTP or an operator.
#[derive(Debug)]
struct SteppedWallClock {
    wall_ms: AtomicI64,
    mono_ms: AtomicU64,
}

impl Clock for SteppedWallClock {
    fn now(&self) -> TimestampMs {
        time::from_millis(self.wall_ms.load(Ordering::SeqCst)).unwrap()
    }

    fn elapsed(&self) -> Duration {
        Duration::from_millis(self.mono_ms.load(Ordering::SeqCst))
    }
}

#[tokio::test]
async fn buckets_follow_the_monotonic_reading_not_the_wall_clock() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(1)).await;
        let clock = Arc::new(SteppedWallClock {
            wall_ms: AtomicI64::new(T0_MS),
            mono_ms: AtomicU64::new(0),
        });
        let limiter = IpLimiter::new(settings, clock.clone());
        assert!(limiter.check(client("192.0.2.1")).is_ok());

        // Wall clock stepped forward a day: no refill.
        clock.wall_ms.fetch_add(86_400_000, Ordering::SeqCst);
        assert!(limiter.check(client("192.0.2.1")).is_err());
        // Stepped back two days: no change either.
        clock.wall_ms.fetch_sub(2 * 86_400_000, Ordering::SeqCst);
        assert_eq!(
            limiter.check(client("192.0.2.1")).unwrap_err(),
            MINUTE,
            "{}",
            db.backend
        );

        // Only monotonic time refills.
        clock.mono_ms.fetch_add(60_000, Ordering::SeqCst);
        assert!(limiter.check(client("192.0.2.1")).is_ok());
    })
    .await;
}

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
async fn proxy_fallback_shares_one_bucket_with_ten_times_the_quota() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(2)).await;
        let limiter = IpLimiter::new(settings, clock());

        let ok = (0..30)
            .filter(|i| limiter.check(proxy(&format!("10.0.0.{i}"))).is_ok())
            .count();

        assert_eq!(ok, 2 * FALLBACK_FACTOR as usize);
        assert_eq!(
            limiter.tracked_clients(),
            0,
            "fallback is not a per-client bucket"
        );
        // The same address seen as a real client has its own bucket.
        assert_eq!(allowed(&limiter, "10.0.0.1", 3), 2);
    })
    .await;
}

fn proxy(addr: &str) -> ClientAddr {
    ClientAddr {
        is_proxy_fallback: true,
        ..client(addr)
    }
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

#[tokio::test]
async fn over_limit_reports_a_denied_key_without_taking_a_token() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(2)).await;
        let clock = clock();
        let limiter = IpLimiter::new(settings, clock.clone());
        let a = client("192.0.2.1");

        assert_eq!(allowed(&limiter, "192.0.2.1", 2), 2);
        // Bucket empty, but no check was denied yet: GCRA offers no peek.
        assert_eq!(limiter.over_limit(a), None);
        let wait = limiter.check_recorded(a).unwrap_err();
        assert_eq!(limiter.over_limit(a), Some(wait), "{}", db.backend);
        // Peeking repeatedly takes nothing and counts down to the same deadline.
        clock.advance(Duration::from_secs(10));
        assert_eq!(limiter.over_limit(a), Some(wait - Duration::from_secs(10)));
        assert_eq!(limiter.over_limit(client("192.0.2.2")), None);

        clock.advance(wait);
        assert_eq!(limiter.over_limit(a), None);
        assert!(limiter.check(a).is_ok(), "the peeks took no token");
    })
    .await;
}

#[tokio::test]
async fn plain_check_denials_share_the_quota_but_are_not_recorded() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(1)).await;
        let limiter = IpLimiter::new(settings, clock());
        let a = client("192.0.2.1");

        limiter.check(a).unwrap();
        let wait = limiter.check(a).unwrap_err();

        assert_eq!(limiter.over_limit(a), None, "{}", db.backend);
        assert_eq!(limiter.denied_clients(), 0);
        assert_eq!(
            limiter.check_recorded(a),
            Err(wait),
            "same bucket, same quota"
        );
        assert_eq!(limiter.over_limit(a), Some(wait));
    })
    .await;
}

#[tokio::test]
async fn over_limit_follows_the_bucket_key_and_the_fallback_bucket() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(1)).await;
        let limiter = IpLimiter::new(settings, clock());
        let fallback = |addr: &str| ClientAddr {
            addr: addr.parse().unwrap(),
            is_proxy_fallback: true,
        };

        limiter.check(client("2001:db8:1:2::1")).unwrap();
        assert!(limiter.check_recorded(client("2001:db8:1:2::1")).is_err());
        for _ in 0..FALLBACK_FACTOR {
            limiter.check(fallback("10.0.0.1")).unwrap();
        }
        assert!(limiter.check_recorded(fallback("10.0.0.1")).is_err());

        assert!(
            limiter.over_limit(client("2001:db8:1:2::ffff")).is_some(),
            "same /64"
        );
        assert!(limiter.over_limit(client("2001:db8:1:3::1")).is_none());
        assert!(
            limiter.over_limit(fallback("10.0.0.2")).is_some(),
            "one fallback bucket"
        );
        assert!(limiter.over_limit(client("10.0.0.1")).is_none());
    })
    .await;
}

#[tokio::test]
async fn denials_are_swept_and_dropped_on_reconfiguration() {
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.ip_rate_limit.per_minute", json!(1)).await;
        let clock = clock();
        let limiter = IpLimiter::new(settings.clone(), clock.clone());
        let a = client("192.0.2.1");
        limiter.check(a).unwrap();
        let _ = limiter.check_recorded(a).unwrap_err();
        assert_eq!(limiter.denied_clients(), 1);

        clock.advance(MINUTE);
        limiter.sweep();
        assert_eq!(limiter.denied_clients(), 0, "expired denials are swept");

        limiter.check(a).unwrap();
        let _ = limiter.check_recorded(a).unwrap_err();
        set(&settings, "auth.ip_rate_limit.per_minute", json!(5)).await;
        assert_eq!(
            limiter.over_limit(a),
            None,
            "a new quota starts every bucket full"
        );
    })
    .await;
}
