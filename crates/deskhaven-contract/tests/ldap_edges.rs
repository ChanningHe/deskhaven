//! `/api/login` edge cases of directory accounts (06 §3.5, §3.6) through the real client
//! API and the fake directory: `auth.tfa.enforce` for a directory account that has not
//! enrolled (403, counted), a promotion while the login is in flight (the second factor is
//! judged on the committed role), and a directory that goes silent after the password was
//! sent (503, but counted).
#![allow(clippy::unwrap_used, clippy::panic)]

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use std::time::Duration;

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::mirror::ErrorBody;
use deskhaven_contract::requests::login;
use deskhaven_core::ldap::{DirectoryLimits, NewDirectory};
use deskhaven_core::users::{Credential, NewUser, Role};
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD, group_dn};
use serde_json::json;

/// `TFA_SETUP_REQUIRED_MESSAGE` of the client API.
const SETUP_REQUIRED: &str =
    "Two-factor authentication is required: set it up in the web console first";

async fn app(fake: &FakeDirectory, limits: DirectoryLimits) -> TestApp {
    let options = TestOptions {
        ldap: Some(fake.connector()),
        ldap_limits: Some(limits),
        ..TestOptions::default()
    };
    let app = TestApp::start_with_options(options).await.unwrap();
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
    new.admin_group_dn = Some(group_dn("admins"));
    app.state.ldap.create_directory(new).await.unwrap();
    app
}

fn text(reply: &Reply) -> String {
    String::from_utf8_lossy(&reply.body).into_owned()
}

fn error_of(reply: &Reply) -> String {
    reply.json::<ErrorBody>().unwrap().error
}

#[tokio::test]
async fn an_unenrolled_directory_account_under_enforcement_is_refused_and_counted() {
    let fake = FakeDirectory::new();
    fake.add_user("alice");
    let app = app(&fake, DirectoryLimits::default()).await;
    app.set_setting("auth.tfa.enforce", json!("all"))
        .await
        .unwrap();
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();

    let refused = app.send(&login("alice", USER_PASSWORD)).await.unwrap();
    let locked = app.send(&login("alice", USER_PASSWORD)).await.unwrap();

    assert_eq!(refused.status, 403, "{}", text(&refused));
    assert_eq!(error_of(&refused), SETUP_REQUIRED);
    assert_eq!(locked.status, 429, "{}", text(&locked));
}

#[tokio::test]
async fn a_promotion_during_the_login_is_judged_on_the_committed_role() {
    let fake = FakeDirectory::new();
    let dn = fake.add_user("alice");
    fake.set_members(&group_dn("admins"), &[&dn]);
    let app = app(&fake, DirectoryLimits::default()).await;
    let alice = app
        .state
        .users
        .create(NewUser::new("alice", Credential::Ldap))
        .await
        .unwrap();
    app.set_setting("auth.tfa.enforce", json!("admins"))
        .await
        .unwrap();
    fake.set_bind_delay(Some(Duration::from_millis(300)));

    // The login read alice as a user; the console promotes her while the bind is pending.
    let request = login("alice", USER_PASSWORD);
    let (reply, ()) = tokio::join!(app.send(&request), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        app.state
            .users
            .set_role(alice.id, Role::Admin)
            .await
            .unwrap();
    });

    let reply = reply.unwrap();
    assert_eq!(reply.status, 403, "{}", text(&reply));
    assert_eq!(error_of(&reply), SETUP_REQUIRED);
}

#[tokio::test]
async fn a_directory_silent_after_the_password_was_sent_counts_the_attempt() {
    let fake = FakeDirectory::new();
    fake.add_user("alice");
    let limits = DirectoryLimits {
        login_timeout: Duration::from_millis(200),
        ..DirectoryLimits::default()
    };
    let app = app(&fake, limits).await;
    assert_eq!(
        app.send(&login("alice", USER_PASSWORD))
            .await
            .unwrap()
            .status,
        200
    );
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();
    fake.set_bind_delay(Some(Duration::from_secs(5)));

    let silent = app.send(&login("alice", USER_PASSWORD)).await.unwrap();
    let next = app.send(&login("alice", USER_PASSWORD)).await.unwrap();

    assert_eq!(silent.status, 503, "{}", text(&silent));
    assert_eq!(error_of(&silent), "Please try later");
    // Counted: the one allowed failure is spent.
    assert_eq!(next.status, 429, "{}", text(&next));
}
