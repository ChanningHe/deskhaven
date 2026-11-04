//! "Test connection" / "test user" diagnostics (M3-09 builds on them): one structured
//! result per step, later steps skipped after a failure, escaped search input, and no
//! password in any result or log line. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::capture_logs;
use common::ldap::Env;
use common::ldap_fake::{SERVICE_PASSWORD, USER_BASE, USER_PASSWORD, group_dn};
use deskhaven_core::ldap::{Diagnostic, DirectoryUpdate, Step, StepStatus};
use deskhaven_core::users::Password;
use deskhaven_db::testing::{TestDb, each_migrated_backend};

fn statuses(diagnostic: &Diagnostic) -> Vec<(Step, StepStatus)> {
    diagnostic
        .steps
        .iter()
        .map(|s| (s.step, s.status))
        .collect()
}

#[tokio::test]
async fn connection_test_reports_connect_and_service_bind() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let settings = env.ldap.stored_settings(env.directory.id).await.unwrap();

        let ok = env.ldap.test_connection(&settings).await;
        env.fake.set_down(true);
        let down = env.ldap.test_connection(&settings).await;

        assert!(ok.passed(), "{}: {ok:?}", db.backend);
        assert_eq!(
            statuses(&ok),
            [
                (Step::Connect, StepStatus::Passed),
                (Step::ServiceBind, StepStatus::Passed)
            ]
        );
        assert!(!down.passed());
        assert_eq!(
            statuses(&down),
            [
                (Step::Connect, StepStatus::Failed),
                (Step::ServiceBind, StepStatus::Skipped)
            ]
        );
        assert!(
            down.steps[0].message.contains("connection refused"),
            "{down:?}"
        );
    })
    .await;
}

#[tokio::test]
async fn anonymous_directories_skip_the_service_bind() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let anonymous = DirectoryUpdate {
            bind_dn: Some(None),
            bind_password: Some(None),
            ..DirectoryUpdate::default()
        };
        env.ldap
            .update_directory(env.directory.id, anonymous)
            .await
            .unwrap();
        let settings = env.ldap.stored_settings(env.directory.id).await.unwrap();

        let result = env.ldap.test_connection(&settings).await;

        assert!(result.passed(), "{}", db.backend);
        assert_eq!(result.steps[1].status, StepStatus::Skipped);
        assert!(env.fake.binds().is_empty());
    })
    .await;
}

#[tokio::test]
async fn user_test_reports_every_step_and_the_mapping() {
    each_migrated_backend(async |db: &TestDb| {
        let admins = group_dn("admins");
        let admin_dn = admins.clone();
        let env = Env::with(db, move |d| d.admin_group_dn = Some(admin_dn)).await;
        let dn = env.fake.add_user("alice");
        env.group("ops").await;
        env.fake.set_members(&group_dn("ops"), &[&dn]);
        env.fake.set_members(&group_dn("unmapped"), &[&dn]);
        env.fake.set_members(&admins, &[&dn]);
        let settings = env.ldap.stored_settings(env.directory.id).await.unwrap();

        let no_password = env.ldap.test_user(&settings, "alice", None).await;
        let right = Password::new(USER_PASSWORD);
        let with_password = env.ldap.test_user(&settings, "alice", Some(&right)).await;
        let wrong = Password::new("wrong-one");
        let wrong = env.ldap.test_user(&settings, "alice", Some(&wrong)).await;
        let unknown = env.ldap.test_user(&settings, "nobody", None).await;

        use StepStatus::{Failed, Passed, Skipped};
        assert_eq!(
            statuses(&no_password),
            [
                (Step::Connect, Passed),
                (Step::ServiceBind, Passed),
                (Step::UserSearch, Passed),
                (Step::UserBind, Skipped),
                (Step::GroupMapping, Passed),
            ],
            "{}",
            db.backend
        );
        assert!(no_password.passed());
        let user = no_password.user.unwrap();
        assert_eq!(user.dn, dn);
        assert_eq!(user.username.as_deref(), Some("alice"));
        assert_eq!(user.email.as_deref(), Some("alice@example.org"));
        assert_eq!(user.display_name.as_deref(), Some("User alice"));
        assert_eq!(user.directory_groups.len(), 3);
        assert_eq!(user.mapped_groups, ["ops"]);
        assert_eq!(user.admin, Some(true));
        assert_eq!(with_password.steps[3].status, Passed);
        assert_eq!(wrong.steps[3].status, Failed);
        assert!(!wrong.passed());
        assert_eq!(unknown.steps[2].status, Failed);
        assert_eq!(unknown.steps[3].status, Skipped);
        assert!(unknown.user.is_none());
    })
    .await;
}

#[tokio::test]
async fn user_test_input_is_escaped_in_the_search_filter() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.fake.add_user("alice");
        let settings = env.ldap.stored_settings(env.directory.id).await.unwrap();

        let result = env.ldap.test_user(&settings, "*)(uid=*\\\0", None).await;

        let searches = env.fake.searches();
        assert_eq!(searches.len(), 1, "{}", db.backend);
        assert_eq!(searches[0].base, USER_BASE);
        assert_eq!(
            searches[0].filter,
            "(&(objectClass=inetOrgPerson)(uid=\\2a\\29\\28uid=\\2a\\5c\\00))"
        );
        assert_eq!(result.steps[2].status, StepStatus::Failed);
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn no_password_reaches_a_result_or_a_log_line() {
    let logs = capture_logs();
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |d| {
            d.bind_password = Some(format!("{SERVICE_PASSWORD}-old"))
        })
        .await;
        env.fake.add_user("alice");
        let settings = env.ldap.stored_settings(env.directory.id).await.unwrap();
        let typed = Password::new("typed-Secret-55");

        let failed_bind = env.ldap.test_connection(&settings).await;
        let user = env.ldap.test_user(&settings, "alice", Some(&typed)).await;
        let login = env.login("alice", "typed-Secret-55").await;

        assert_eq!(
            failed_bind.steps[1].status,
            StepStatus::Failed,
            "{}",
            db.backend
        );
        assert!(login.is_err());
        let shown = format!("{failed_bind:?} {user:?} {login:?} {settings:?}");
        for secret in [SERVICE_PASSWORD, "typed-Secret-55"] {
            assert!(!shown.contains(secret), "{shown}");
        }
    })
    .await;
    let logged = logs.take();
    assert!(!logged.contains(SERVICE_PASSWORD), "{logged}");
    assert!(!logged.contains("typed-Secret-55"), "{logged}");
}
