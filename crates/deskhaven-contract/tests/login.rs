//! Multi-step account flows (03 §3.2-3.4): login -> currentUser -> logout, request
//! boundary checks, lockout and enumeration resistance across attempts, hasher overload and
//! a credential change during login. Every reply is decoded with the client mirror types.
//! Per-IP bearer limiting: `tests/bearer_limits.rs`; login events: `tests/login_events.rs`.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::AUTH_LOCKOUT_MAX_FAILURES;
use deskhaven_contract::app::{DEFAULT_PEER, TestApp};
use deskhaven_contract::check::Reply;
use deskhaven_contract::fixture::Body;
use deskhaven_contract::mirror::{ClientUserPayload, ErrorBody, LoginResponse};
use deskhaven_contract::requests::{bearer, device_info, from, login};
use deskhaven_contract::seeds::{
    SEED_PASSWORD, SEED_RUSTDESK_ID, SEED_USERNAME, SEED_UUID, Seeds, UNKNOWN_USERNAME,
};
use deskhaven_core::users::{HashLimits, HashParams, Password, PasswordHasher, Status};
use deskhaven_db::entity::login_event;
use sea_orm::{ConnectionTrait, EntityTrait, PaginatorTrait};
use serde_json::json;

const OTHER_PEER: &str = "198.51.100.20:40000";
const WRONG_PASSWORD: &str = "not-the-password";

fn error_of(reply: &Reply) -> String {
    reply.json::<ErrorBody>().unwrap().error
}

fn retry_after(reply: &Reply) -> u64 {
    let value = reply.headers.get("retry-after").unwrap();
    value.to_str().unwrap().parse().unwrap()
}

async fn seeded(app: TestApp) -> TestApp {
    Seeds::builtin()
        .apply(&["local-user".to_owned()], &app.state)
        .await
        .unwrap();
    app
}

async fn app_with_user() -> TestApp {
    seeded(TestApp::start().await.unwrap()).await
}

async fn access_token(app: &TestApp) -> String {
    let reply = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let body: LoginResponse = reply.json().unwrap();
    assert_eq!(body.kind.as_deref(), Some("access_token"));
    body.access_token.unwrap()
}

async fn event_rows(app: &TestApp) -> u64 {
    login_event::Entity::find()
        .count(&app.state.db)
        .await
        .unwrap()
}

#[tokio::test]
async fn login_token_authenticates_current_user_and_binds_the_device() {
    let app = app_with_user().await;

    let token = access_token(&app).await;
    let reply = app.send(&bearer("/api/currentUser", &token)).await.unwrap();

    assert_eq!(reply.status, 200);
    let user: ClientUserPayload = reply.json().unwrap();
    assert_eq!(user.name, SEED_USERNAME);
    let user_id = app
        .state
        .users
        .get_by_username(SEED_USERNAME)
        .await
        .unwrap()
        .id;
    let sessions = app.state.sessions.list_for_user(user_id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    let device = sessions[0].device.clone().unwrap();
    assert_eq!(
        (device.rustdesk_id.as_str(), device.uuid.as_str()),
        (SEED_RUSTDESK_ID, SEED_UUID)
    );
    assert_eq!(sessions[0].device_info, Some(device_info()));
    assert_eq!(sessions[0].ip, Some(DEFAULT_PEER.ip()));
}

#[tokio::test]
async fn logout_revokes_the_presented_token_and_the_next_use_is_401() {
    let app = app_with_user().await;
    let token = access_token(&app).await;
    let other = access_token(&app).await;

    let logout = app.send(&bearer("/api/logout", &token)).await.unwrap();
    let after = app.send(&bearer("/api/currentUser", &token)).await.unwrap();
    let again = app.send(&bearer("/api/logout", &token)).await.unwrap();
    let untouched = app.send(&bearer("/api/currentUser", &other)).await.unwrap();

    assert_eq!((logout.status, logout.body.len()), (200, 0));
    assert_eq!(
        (after.status, error_of(&after)),
        (401, "Unauthorized".to_owned())
    );
    assert_eq!(again.status, 401);
    assert_eq!(
        untouched.status, 200,
        "only the presented session is revoked"
    );
}

#[tokio::test]
async fn current_user_rejects_a_user_disabled_without_an_event() {
    let app = app_with_user().await;
    let token = access_token(&app).await;
    let cached = app.send(&bearer("/api/currentUser", &token)).await.unwrap();
    assert_eq!(cached.status, 200);

    // Behind the user service's back: no commit hook, no event, the session stays cached.
    app.state
        .db
        .execute_unprepared("UPDATE users SET status = 'disabled' WHERE username = 'alice'")
        .await
        .unwrap();
    let reply = app.send(&bearer("/api/currentUser", &token)).await.unwrap();

    assert_eq!(
        (reply.status, error_of(&reply)),
        (401, "Unauthorized".to_owned())
    );
}

#[tokio::test]
async fn login_requires_a_json_content_type() {
    let app = app_with_user().await;
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();
    let with_type = |value: Option<&str>| {
        let mut request = login(SEED_USERNAME, WRONG_PASSWORD);
        request.headers.remove("content-type");
        if let Some(value) = value {
            request
                .headers
                .insert("content-type".to_owned(), value.to_owned());
        }
        request
    };

    for refused in [
        Some("text/plain; charset=utf-8"),
        Some("application/x-www-form-urlencoded"),
        None,
    ] {
        let reply = app.send(&with_type(refused)).await.unwrap();
        assert_eq!(
            (reply.status, error_of(&reply)),
            (400, "Invalid login request".to_owned()),
            "{refused:?}"
        );
    }
    let mut accepted = login(SEED_USERNAME, SEED_PASSWORD);
    accepted.headers.insert(
        "content-type".to_owned(),
        "Application/JSON; charset=UTF-8".to_owned(),
    );
    let reply = app.send(&accepted).await.unwrap();

    // With one failure allowed, a counted refusal would have locked the account.
    assert_eq!(reply.status, 200, "the refused requests were not counted");
    assert_eq!(event_rows(&app).await, 1, "and not recorded");
}

#[tokio::test]
async fn oversized_login_body_is_413_before_parsing() {
    let app = app_with_user().await;
    let mut request = login(SEED_USERNAME, SEED_PASSWORD);
    // Valid JSON with a huge deviceInfo: parsed, it would be a 400 (info too large).
    request.body = Body::Json(json!({
        "username": SEED_USERNAME, "password": SEED_PASSWORD,
        "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID, "type": "account",
        "deviceInfo": { "name": "x".repeat(deskhaven_client_api::MAX_LOGIN_BODY) },
    }));

    let reply = app.send(&request).await.unwrap();

    assert_eq!(reply.status, 413);
    assert_eq!(error_of(&reply), "payload too large");
    assert_eq!(event_rows(&app).await, 0);
}

#[tokio::test]
async fn unknown_and_existing_usernames_get_identical_replies_until_and_after_lockout() {
    let app = app_with_user().await;
    let budget = app
        .state
        .settings
        .snapshot()
        .get(&AUTH_LOCKOUT_MAX_FAILURES);

    for _ in 0..budget {
        let existing = app
            .send(&login(SEED_USERNAME, WRONG_PASSWORD))
            .await
            .unwrap();
        let unknown = app
            .send(&login(UNKNOWN_USERNAME, WRONG_PASSWORD))
            .await
            .unwrap();
        assert_eq!((existing.status, &existing.body), (401, &unknown.body));
        assert_eq!(unknown.status, 401);
    }
    let existing = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();
    let unknown = app
        .send(&login(UNKNOWN_USERNAME, WRONG_PASSWORD))
        .await
        .unwrap();

    assert_eq!((existing.status, unknown.status), (429, 429));
    assert_eq!(existing.body, unknown.body);
    assert_eq!(error_of(&existing), "Please try later");
    assert!(retry_after(&existing) > 0);
    let elsewhere = app
        .send(&from(login(SEED_USERNAME, SEED_PASSWORD), OTHER_PEER))
        .await
        .unwrap();
    assert_eq!(elsewhere.status, 200, "the lock is per source");
}

/// One slot, Argon2 slow enough (hundreds of ms) that one computation reliably outlasts the
/// few database statements a test runs meanwhile.
fn slow_single_slot_hasher(acquire_timeout: Duration) -> PasswordHasher {
    let params = HashParams {
        iterations: 24,
        ..HashParams::DEFAULT
    };
    let limits = HashLimits {
        max_concurrent: 1,
        acquire_timeout,
    };
    PasswordHasher::with_limits(params, limits).unwrap()
}

/// Waits until `hasher`'s only slot is taken (or `task` ended without taking it).
async fn until_slot_taken<T>(hasher: &PasswordHasher, task: &tokio::task::JoinHandle<T>) {
    while hasher.available_slots() > 0 && !task.is_finished() {
        tokio::task::yield_now().await;
    }
    assert_eq!(hasher.available_slots(), 0, "the computation never started");
}

#[tokio::test]
async fn hasher_busy_is_503_with_retry_after_for_known_and_unknown_users_and_is_refunded() {
    let hasher = slow_single_slot_hasher(Duration::ZERO);
    let app = seeded(TestApp::start_with_hasher(hasher.clone()).await.unwrap()).await;
    // One failure locks: a counted busy attempt would make the next reply a 429.
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();

    let holder = tokio::spawn({
        let hasher = hasher.clone();
        async move { hasher.verify_dummy(&Password::new("x")).await }
    });
    until_slot_taken(&hasher, &holder).await;
    let busy_unknown = app
        .send(&login(UNKNOWN_USERNAME, WRONG_PASSWORD))
        .await
        .unwrap();
    let busy_existing = app
        .send(&login(SEED_USERNAME, WRONG_PASSWORD))
        .await
        .unwrap();
    holder.await.unwrap().unwrap();

    for busy in [&busy_unknown, &busy_existing] {
        assert_eq!(busy.status, 503, "{}", String::from_utf8_lossy(&busy.body));
        assert_eq!(error_of(busy), "Please try later");
        assert!(retry_after(busy) > 0);
    }
    assert_eq!(event_rows(&app).await, 0, "busy attempts are not recorded");
    for name in [UNKNOWN_USERNAME, SEED_USERNAME] {
        let first = app.send(&login(name, WRONG_PASSWORD)).await.unwrap();
        let second = app.send(&login(name, WRONG_PASSWORD)).await.unwrap();
        assert_eq!((first.status, second.status), (401, 429), "{name}");
    }
}

#[tokio::test]
async fn credential_change_during_login_is_401_and_counted() {
    let hasher = slow_single_slot_hasher(Duration::from_secs(30));
    let app = Arc::new(seeded(TestApp::start_with_hasher(hasher.clone()).await.unwrap()).await);
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();
    let alice = app
        .state
        .users
        .get_by_username(SEED_USERNAME)
        .await
        .unwrap()
        .id;

    // The login has read the account and is verifying the (right) password ...
    let pending = tokio::spawn({
        let app = Arc::clone(&app);
        async move {
            app.send(&login(SEED_USERNAME, SEED_PASSWORD))
                .await
                .unwrap()
        }
    });
    until_slot_taken(&hasher, &pending).await;
    // ... when the account changes: its credential stamp no longer matches.
    app.state
        .users
        .set_status(alice, Status::Disabled)
        .await
        .unwrap();
    let raced = pending.await.unwrap();
    let next = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();

    assert_eq!(
        (raced.status, error_of(&raced)),
        (401, "Wrong credentials".to_owned())
    );
    assert!(
        app.state
            .sessions
            .list_for_user(alice)
            .await
            .unwrap()
            .is_empty()
    );
    // Not counted, the next reply would be the disabled 403; counted, the account is locked.
    assert_eq!(next.status, 429);
}
