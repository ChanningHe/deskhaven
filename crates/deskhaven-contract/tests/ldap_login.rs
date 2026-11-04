//! `/api/login` with directory accounts (06 §3.5) through the real client API and the fake
//! directory: the directory password logs in, wrong and unknown answer the same 401 and
//! are counted, an unreachable directory is a 503 that is neither counted nor answered by
//! a local password.
#![allow(clippy::unwrap_used, clippy::panic)]

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::mirror::{ErrorBody, LoginResponse};
use deskhaven_contract::requests::login;
use deskhaven_core::ldap::NewDirectory;
use deskhaven_core::users::AuthSource;
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD};
use serde_json::json;

async fn app(fake: &FakeDirectory) -> TestApp {
    let options = TestOptions {
        ldap: Some(fake.connector()),
        ..TestOptions::default()
    };
    let app = TestApp::start_with_options(options).await.unwrap();
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
    app.state.ldap.create_directory(new).await.unwrap();
    app
}

fn error_of(reply: &Reply) -> String {
    reply.json::<ErrorBody>().unwrap().error
}

#[tokio::test]
async fn the_directory_password_logs_a_directory_account_in() {
    let fake = FakeDirectory::new();
    fake.add_user("alice");
    let app = app(&fake).await;

    let reply = app.send(&login("alice", USER_PASSWORD)).await.unwrap();

    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let body: LoginResponse = reply.json().unwrap();
    assert_eq!(body.kind.as_deref(), Some("access_token"));
    assert_eq!(body.user.unwrap().name, "alice");
    let account = app.state.users.get_by_username("alice").await.unwrap();
    assert_eq!(account.auth_source, AuthSource::Ldap);
}

#[tokio::test]
async fn wrong_and_unknown_answer_the_same_and_are_counted() {
    let fake = FakeDirectory::new();
    fake.add_user("alice");
    let app = app(&fake).await;
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();

    let wrong = app.send(&login("alice", "not-the-password")).await.unwrap();
    let unknown = app.send(&login("mallory", USER_PASSWORD)).await.unwrap();
    let locked = app.send(&login("alice", USER_PASSWORD)).await.unwrap();

    for reply in [&wrong, &unknown] {
        assert_eq!(reply.status, 401);
        assert_eq!(error_of(reply), "Wrong credentials");
    }
    assert_eq!(locked.status, 429);
}

#[tokio::test]
async fn an_unreachable_directory_is_a_503_that_counts_nothing() {
    let fake = FakeDirectory::new();
    fake.add_user("alice");
    let app = app(&fake).await;
    app.send(&login("alice", USER_PASSWORD)).await.unwrap();
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();
    fake.set_down(true);

    let first = app.send(&login("alice", USER_PASSWORD)).await.unwrap();
    let second = app.send(&login("alice", "anything")).await.unwrap();
    fake.set_down(false);
    let back = app.send(&login("alice", USER_PASSWORD)).await.unwrap();

    for reply in [&first, &second] {
        assert_eq!(
            reply.status,
            503,
            "{}",
            String::from_utf8_lossy(&reply.body)
        );
        assert_eq!(error_of(reply), "Please try later");
        assert!(reply.headers.contains_key("retry-after"));
    }
    assert_eq!(back.status, 200, "{}", String::from_utf8_lossy(&back.body));
}
