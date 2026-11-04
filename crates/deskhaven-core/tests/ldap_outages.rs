//! Directory trouble during logins (06 §3.5, §3.6): busy / unavailable result codes,
//! stalled directories (one total timeout per login) and the bound on concurrent directory
//! logins. Directory accounts get `DirectoryUnavailable` (503, not counted); unknown names
//! the same `InvalidCredentials` a wrong local password gets. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::ldap::Env;
use common::ldap_fake::USER_PASSWORD;
use deskhaven_core::ldap::DirectoryLimits;
use deskhaven_core::users::{AuthError, Password};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

/// LDAP `busy`, `unavailable`, `other`: the directory could not decide.
const UNAVAILABLE_CODES: [u32; 3] = [51, 52, 80];
/// LDAP `unwillingToPerform`: a decision against the login.
const UNWILLING: u32 = 53;

#[tokio::test]
async fn busy_and_unavailable_bind_results_are_outages_other_refusals_wrong_passwords() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        env.ldap_user("alice").await;

        for code in UNAVAILABLE_CODES {
            env.fake.set_bind_refusal(Some(code));
            let outcome = env.login("alice", USER_PASSWORD).await;
            assert!(
                matches!(outcome, Err(AuthError::DirectoryUnavailable)),
                "{}: {code}: {outcome:?}",
                db.backend
            );
        }
        env.fake.set_bind_refusal(Some(UNWILLING));
        let unwilling = env.login("alice", USER_PASSWORD).await;
        assert!(
            matches!(unwilling, Err(AuthError::InvalidCredentials)),
            "{unwilling:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_stalled_directory_times_out_and_concurrent_logins_are_bounded() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        env.ldap_user("alice").await;
        let limits = DirectoryLimits {
            concurrent: 1,
            queue_timeout: Duration::from_millis(50),
            login_timeout: Duration::from_millis(300),
        };
        let ldap = env.ldap.clone().with_limits(limits);
        env.fake.set_hang(true);
        let password = Password::new(USER_PASSWORD);
        let started = Instant::now();

        let (stalled, queued) =
            tokio::join!(ldap.authenticate_password("alice", &password), async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                ldap.authenticate_password("alice", &password).await
            });
        let unknown = ldap.authenticate_password("nobody", &password).await;

        assert!(
            matches!(stalled, Err(AuthError::DirectoryUnavailable)),
            "{}: {stalled:?}",
            db.backend
        );
        assert!(
            matches!(queued, Err(AuthError::DirectoryUnavailable)),
            "{queued:?}"
        );
        assert!(
            matches!(unknown, Err(AuthError::InvalidCredentials)),
            "{unknown:?}"
        );
        // The queued login never reached the directory.
        assert_eq!(env.fake.connects(), 2);
        assert!(started.elapsed() < Duration::from_secs(5));
    })
    .await;
}
