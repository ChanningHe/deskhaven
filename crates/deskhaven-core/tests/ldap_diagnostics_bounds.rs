//! Bounds of the LDAP diagnostics (M3-09): one total timeout that records the steps not
//! finished as timed out, the directory slots shared with logins, and `attempted` (would a
//! login have counted?) for the console's lockout settling. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::{Duration, Instant};

use common::ldap::Env;
use common::ldap_fake::USER_PASSWORD;
use deskhaven_core::ldap::{Diagnostic, DirectoryLimits, Step, StepStatus};
use deskhaven_core::users::Password;
use deskhaven_db::testing::{TestDb, each_migrated_backend};

fn statuses(diagnostic: &Diagnostic) -> Vec<(Step, StepStatus)> {
    diagnostic
        .steps
        .iter()
        .map(|s| (s.step, s.status))
        .collect()
}

fn timeout_after(diagnostic_timeout: Duration) -> DirectoryLimits {
    DirectoryLimits {
        diagnostic_timeout,
        ..DirectoryLimits::default()
    }
}

#[tokio::test]
async fn a_stalled_directory_ends_the_test_with_the_remaining_steps_timed_out() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        let ldap = env
            .ldap
            .clone()
            .with_limits(timeout_after(Duration::from_millis(200)));
        let settings = ldap.stored_settings(env.directory.id).await.unwrap();
        let password = Password::new(USER_PASSWORD);
        let started = Instant::now();

        env.fake.set_bind_delay(Some(Duration::from_secs(5)));
        let slow_bind = ldap.test_user(&settings, "alice", Some(&password)).await;
        env.fake.set_bind_delay(None);
        env.fake.set_hang(true);
        let hung = ldap.test_connection(&settings).await;

        use StepStatus::{Failed, Passed};
        assert!(started.elapsed() < Duration::from_secs(3), "{}", db.backend);
        assert_eq!(
            statuses(&slow_bind),
            [
                (Step::Connect, Passed),
                (Step::ServiceBind, Passed),
                (Step::UserSearch, Passed),
                (Step::UserBind, Failed),
                (Step::GroupMapping, Failed),
            ]
        );
        assert!(
            slow_bind.steps[3].message.contains("timed out"),
            "{slow_bind:?}"
        );
        assert!(slow_bind.attempted, "the password was sent");
        assert_eq!(
            statuses(&hung),
            [(Step::Connect, Failed), (Step::ServiceBind, Failed)]
        );
        assert!(hung.steps[0].message.contains("timed out"), "{hung:?}");
        assert!(!hung.attempted);
    })
    .await;
}

#[tokio::test]
async fn attempted_says_whether_a_login_would_have_counted() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        let settings = env.ldap.stored_settings(env.directory.id).await.unwrap();
        let right = Password::new(USER_PASSWORD);
        let wrong = Password::new("wrong-one");

        let refused = env.ldap.test_user(&settings, "alice", Some(&wrong)).await;
        let accepted = env.ldap.test_user(&settings, "alice", Some(&right)).await;
        let missing = env.ldap.test_user(&settings, "nobody", Some(&wrong)).await;
        let no_password = env.ldap.test_user(&settings, "alice", None).await;
        env.fake.set_down(true);
        let down = env.ldap.test_user(&settings, "alice", Some(&wrong)).await;

        assert!(refused.attempted, "{}", db.backend);
        assert!(accepted.attempted);
        assert!(
            missing.attempted,
            "an unknown name counts, like a login's decoy bind"
        );
        assert!(!no_password.attempted);
        assert!(!down.attempted, "the directory never saw the password");
    })
    .await;
}

#[tokio::test]
async fn diagnostics_share_the_directory_slots_with_logins() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        env.ldap_user("alice").await;
        let ldap = env.ldap.clone().with_limits(DirectoryLimits {
            concurrent: 1,
            queue_timeout: Duration::from_millis(50),
            login_timeout: Duration::from_millis(500),
            ..DirectoryLimits::default()
        });
        let settings = ldap.stored_settings(env.directory.id).await.unwrap();
        env.fake.set_hang(true);
        let password = Password::new(USER_PASSWORD);

        let (_, busy) = tokio::join!(ldap.authenticate_password("alice", &password), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            ldap.test_connection(&settings).await
        });

        assert_eq!(busy.steps[0].status, StepStatus::Failed, "{}", db.backend);
        assert!(busy.steps[0].message.contains("busy"), "{busy:?}");
        assert!(!busy.attempted);
    })
    .await;
}
