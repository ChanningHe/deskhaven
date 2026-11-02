//! Multi-step account flows (03 §3.2-3.4): login -> currentUser -> logout, lockout and
//! enumeration resistance across attempts, hasher overload, and the per-IP budget that
//! failed bearer tokens spend. Every reply is decoded with the client mirror types.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::net::SocketAddr;
use std::time::Duration;

use deskhaven_config::settings::AUTH_IP_RATE_LIMIT_PER_MINUTE;
use deskhaven_contract::app::{DEFAULT_PEER, TestApp};
use deskhaven_contract::check::Reply;
use deskhaven_contract::fixture::{Body, Request};
use deskhaven_contract::mirror::{ClientUserPayload, ErrorBody, LoginOptions, LoginResponse};
use deskhaven_contract::seeds::{
    SEED_PASSWORD, SEED_RUSTDESK_ID, SEED_USERNAME, SEED_UUID, Seeds, UNKNOWN_USERNAME,
};
use deskhaven_core::users::{HashLimits, HashParams, PasswordHasher};
use serde_json::{Value, json};

const OTHER_PEER: &str = "198.51.100.20:40000";
const WRONG_PASSWORD: &str = "not-the-password";
const RANDOM_TOKEN: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";

fn device_info() -> Value {
    json!({ "os": "linux", "type": "client", "name": "flow-host" })
}

/// What `LoginRequest.toJson` sends for an account login (hbbs.dart:155-177).
fn login(username: &str, password: &str) -> Request {
    let mut request = Request::new("POST", "/api/login");
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request.body = Body::Json(json!({
        "username": username, "password": password,
        "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID, "autoLogin": true,
        "type": "account", "deviceInfo": device_info(),
    }));
    request
}

/// `currentUser` / `logout` as the client sends them (user_model.dart:63-78, 170-183).
fn bearer(path: &str, token: &str) -> Request {
    let mut request = Request::new("POST", path);
    request
        .headers
        .insert("content-type".to_owned(), "application/json".to_owned());
    request
        .headers
        .insert("authorization".to_owned(), format!("Bearer {token}"));
    request.body = Body::Json(json!({ "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID }));
    request
}

fn from(mut request: Request, peer: &str) -> Request {
    request.peer = Some(peer.parse::<SocketAddr>().unwrap());
    request
}

fn error_of(reply: &Reply) -> String {
    reply.json::<ErrorBody>().unwrap().error
}

fn retry_after(reply: &Reply) -> u64 {
    let value = reply.headers.get("retry-after").unwrap();
    value.to_str().unwrap().parse().unwrap()
}

async fn app_with_user() -> TestApp {
    let app = TestApp::start().await.unwrap();
    Seeds::builtin()
        .apply(&["local-user".to_owned()], &app.state)
        .await
        .unwrap();
    app
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
async fn login_accepts_a_json_body_whatever_the_content_type() {
    let app = app_with_user().await;
    let mut request = login(SEED_USERNAME, SEED_PASSWORD);
    request.headers.insert(
        "content-type".to_owned(),
        "text/plain; charset=utf-8".to_owned(),
    );

    let reply = app.send(&request).await.unwrap();

    assert_eq!(reply.status, 200);
}

#[tokio::test]
async fn unknown_and_existing_usernames_get_identical_replies_until_and_after_lockout() {
    let app = app_with_user().await;
    let budget = app
        .state
        .settings
        .snapshot()
        .get(&deskhaven_config::settings::AUTH_LOCKOUT_MAX_FAILURES);

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

/// Argon2 slow enough (hundreds of ms) that one computation reliably outlasts a request.
fn slow_single_slot_hasher() -> PasswordHasher {
    let params = HashParams {
        iterations: 24,
        ..HashParams::DEFAULT
    };
    let limits = HashLimits {
        max_concurrent: 1,
        acquire_timeout: Duration::ZERO,
    };
    PasswordHasher::with_limits(params, limits).unwrap()
}

#[tokio::test]
async fn hasher_busy_is_503_with_retry_after_for_known_and_unknown_users_and_is_refunded() {
    let hasher = slow_single_slot_hasher();
    let app = TestApp::start_with_hasher(hasher.clone()).await.unwrap();
    Seeds::builtin()
        .apply(&["local-user".to_owned()], &app.state)
        .await
        .unwrap();
    // One failure locks: a counted busy attempt would make the next reply a 429.
    app.set_setting("auth.lockout.max_failures", json!(1))
        .await
        .unwrap();

    // Occupy the only slot (acquired when the task is first polled).
    let holder = tokio::spawn({
        let hasher = hasher.clone();
        async move {
            hasher
                .verify_dummy(&deskhaven_core::users::Password::new("x"))
                .await
        }
    });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
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
    for name in [UNKNOWN_USERNAME, SEED_USERNAME] {
        let first = app.send(&login(name, WRONG_PASSWORD)).await.unwrap();
        let second = app.send(&login(name, WRONG_PASSWORD)).await.unwrap();
        assert_eq!((first.status, second.status), (401, 429), "{name}");
    }
}

#[tokio::test]
async fn bearer_failures_spend_the_per_ip_budget_and_then_429_before_any_lookup() {
    let app = app_with_user().await;
    let token = access_token(&app).await;
    let budget = app
        .state
        .settings
        .snapshot()
        .get(&AUTH_IP_RATE_LIMIT_PER_MINUTE);

    // The login above took one token of this IP's budget.
    for i in 1..budget {
        let reply = app
            .send(&bearer("/api/currentUser", RANDOM_TOKEN))
            .await
            .unwrap();
        assert_eq!(reply.status, 401, "request {i}");
    }
    let sprayed = app
        .send(&bearer("/api/currentUser", RANDOM_TOKEN))
        .await
        .unwrap();
    let valid_here = app.send(&bearer("/api/currentUser", &token)).await.unwrap();
    let login_here = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();
    let valid_elsewhere = app
        .send(&from(bearer("/api/currentUser", &token), OTHER_PEER))
        .await
        .unwrap();

    assert_eq!(sprayed.status, 429);
    assert_eq!(error_of(&sprayed), "Please try later");
    assert!(retry_after(&sprayed) > 0);
    // Rejected before the token is looked up: even a valid one gets no answer from here.
    assert_eq!(valid_here.status, 429);
    assert_eq!(login_here.status, 429, "logins share the per-IP budget");
    assert_eq!(valid_elsewhere.status, 200);
}

#[tokio::test]
async fn valid_bearer_requests_do_not_spend_the_per_ip_budget() {
    let app = app_with_user().await;
    let token = access_token(&app).await;
    let budget = app
        .state
        .settings
        .snapshot()
        .get(&AUTH_IP_RATE_LIMIT_PER_MINUTE);

    for _ in 0..budget * 2 {
        let reply = app.send(&bearer("/api/currentUser", &token)).await.unwrap();
        assert_eq!(reply.status, 200);
    }
    let login_after = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();

    assert_eq!(login_after.status, 200);
}

#[tokio::test]
async fn login_options_stays_200_when_the_ip_is_over_its_limit() {
    let app = TestApp::start().await.unwrap();
    let budget = app
        .state
        .settings
        .snapshot()
        .get(&AUTH_IP_RATE_LIMIT_PER_MINUTE);
    for _ in 0..=budget {
        app.send(&bearer("/api/currentUser", RANDOM_TOKEN))
            .await
            .unwrap();
    }
    let limited = app
        .send(&bearer("/api/currentUser", RANDOM_TOKEN))
        .await
        .unwrap();

    let options = app
        .send(&Request::new("GET", "/api/login-options"))
        .await
        .unwrap();

    assert_eq!(limited.status, 429);
    assert_eq!(options.status, 200);
    assert!(options.json::<LoginOptions>().unwrap().is_empty());
}
