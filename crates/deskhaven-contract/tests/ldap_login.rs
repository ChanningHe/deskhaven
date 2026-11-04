//! `/api/login` with directory accounts (06 §3.5) through the real client API and the fake
//! directory: the directory password logs in, wrong and unknown answer the same 401 and
//! are counted, an unreachable directory is a 503 for directory accounts that is neither
//! counted nor answered by a local password (and a counted 401 for unknown names), TOTP
//! applies to directory accounts, login events say `ldap`.
#![allow(clippy::unwrap_used, clippy::panic)]

#[path = "../../deskhaven-core/tests/common/ldap_fake.rs"]
mod ldap_fake;

use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::mirror::{ErrorBody, LoginResponse, SecondFactorChallenge};
use deskhaven_contract::requests::{login, second_step};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::ldap::NewDirectory;
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::users::AuthSource;
use deskhaven_db::entity::login_event::{self, Method};
use ldap_fake::{FakeDirectory, SERVICE_DN, SERVICE_PASSWORD, USER_BASE, USER_PASSWORD};
use sea_orm::EntityTrait;
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

#[tokio::test]
async fn an_outage_answers_unknown_names_like_a_wrong_local_password() {
    let fake = FakeDirectory::new();
    let app = app(&fake).await;
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();
    fake.set_down(true);

    let unknown = app.send(&login("mallory", USER_PASSWORD)).await.unwrap();
    let counted = app.send(&login("mallory", USER_PASSWORD)).await.unwrap();

    assert_eq!(
        unknown.status,
        401,
        "{}",
        String::from_utf8_lossy(&unknown.body)
    );
    assert_eq!(error_of(&unknown), "Wrong credentials");
    assert_eq!(counted.status, 429);
}

#[tokio::test]
async fn an_enrolled_directory_account_needs_the_totp_code() {
    let fake = FakeDirectory::new();
    fake.add_user("alice");
    let app = app(&fake).await;
    let first: LoginResponse = app
        .send(&login("alice", USER_PASSWORD))
        .await
        .unwrap()
        .json()
        .unwrap();
    assert!(first.access_token.is_some());
    let id = app.state.users.get_by_username("alice").await.unwrap().id;
    let tfa = &app.state.tfa;
    let enrolment = tfa.begin_enrolment(id).await.unwrap();
    let secret = TotpSecret::from_base32(&enrolment.secret).unwrap();
    let now = SystemClock.now_ms();
    tfa.confirm_enrolment(id, &totp::code_at(&secret, now))
        .await
        .unwrap();

    let reply = app.send(&login("alice", USER_PASSWORD)).await.unwrap();
    let challenge: SecondFactorChallenge = reply.json().unwrap();
    let code = totp::code_at_step(&secret, totp::step_at(now) + 1);
    let request = second_step(&challenge.secret, &challenge.user.name, &code);
    let done = app.send(&request).await.unwrap();

    assert_eq!(done.status, 200, "{}", String::from_utf8_lossy(&done.body));
    let body: LoginResponse = done.json().unwrap();
    assert_eq!(body.kind.as_deref(), Some("access_token"));
    let methods: Vec<Method> = login_event::Entity::find()
        .all(&app.state.db)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.method)
        .collect();
    // Directory password steps say ldap, the second step its factor.
    assert_eq!(methods, [Method::Ldap, Method::Ldap, Method::Totp]);
}
