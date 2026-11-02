//! Log output of the login throttle (M2-06): lock start and proxy-fallback warnings.
//!
//! Kept in its own test binary where every test installs a capturing subscriber: `tracing`
//! caches callsite interest process-wide, so a callsite first hit by a parallel test thread
//! without a subscriber can miss events in a scoped one.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::IpAddr;

use common::{capture_logs, clock, set, settings};
use deskhaven_core::auth::limiter::{
    AccountLockout, ClientAddr, FALLBACK_WARN_INTERVAL, IpLimiter, Reserved, Subject,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::json;

fn client(addr: &str, is_proxy_fallback: bool) -> ClientAddr {
    ClientAddr {
        addr: addr.parse::<IpAddr>().unwrap(),
        is_proxy_fallback,
    }
}

#[tokio::test]
async fn lock_start_is_logged_without_secrets() {
    let logs = capture_logs();
    each_migrated_backend(async |db: &TestDb| {
        let settings = settings(db).await;
        set(&settings, "auth.lockout.max_failures", json!(2)).await;
        let lockout = AccountLockout::new(db.conn.clone(), settings, clock());
        let alice = Subject::new(client("192.0.2.1", false), "Alice");

        for _ in 0..2 {
            assert!(matches!(
                lockout.reserve(&alice).await.unwrap(),
                Reserved::Granted(_)
            ));
        }

        let out = logs.take();
        assert_eq!(out.matches("login lockout started").count(), 1, "{out}");
        assert!(out.contains("WARN"), "{out}");
        assert!(out.contains("username=\"alice\""), "{out}");
        assert!(out.contains("source=\"192.0.2.1\""), "{out}");
        assert!(out.contains("scope=\"source\""), "{out}");
        assert!(out.contains("level=0"), "{out}");
        assert!(out.contains("duration_s=900"), "{out}");
    })
    .await;
}

#[tokio::test]
async fn proxy_fallback_warning_is_rate_limited() {
    let logs = capture_logs();
    each_migrated_backend(async |db: &TestDb| {
        let clock = clock();
        let limiter = IpLimiter::new(settings(db).await, clock.clone());
        let proxy = client("10.0.0.1", true);

        for _ in 0..50 {
            let _ = limiter.check(proxy);
        }
        let first = logs.take();
        clock.advance(FALLBACK_WARN_INTERVAL);
        let _ = limiter.check(proxy);
        let second = logs.take();

        let warnings = |s: &str| s.matches("trusted proxy address").count();
        assert_eq!(warnings(&first), 1, "{}: {first}", db.backend);
        assert!(first.contains("WARN"), "{first}");
        assert_eq!(warnings(&second), 1, "{}: {second}", db.backend);
    })
    .await;
}
