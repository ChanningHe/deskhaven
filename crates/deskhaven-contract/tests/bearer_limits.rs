//! Per-IP limiting of bearer authentication (`deskhaven_client_api::BearerUser`): rejected
//! tokens spend the address's login budget and, once over it, uncached tokens get a 429
//! before any database lookup; cached valid tokens always pass; login denials never close
//! the bearer gate; `/api/login-options` is never limited.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_config::settings::AUTH_IP_RATE_LIMIT_PER_MINUTE;
use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::fixture::Request;
use deskhaven_contract::mirror::{ErrorBody, LoginOptions, LoginResponse};
use deskhaven_contract::requests::{bearer, from, login};
use deskhaven_contract::seeds::{SEED_PASSWORD, SEED_USERNAME, Seeds};
use deskhaven_core::auth::limiter::FALLBACK_FACTOR;

const DEFAULT: &str = "203.0.113.7:50000";
const OTHER_PEER: &str = "198.51.100.20:40000";
const PROXY_A: &str = "10.0.0.1:40000";
const PROXY_B: &str = "10.0.0.2:40000";
const RANDOM_TOKEN: &str = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx";
const CURRENT_USER: &str = "/api/currentUser";

fn error_of(reply: &Reply) -> String {
    reply.json::<ErrorBody>().unwrap().error
}

async fn seeded(app: TestApp) -> TestApp {
    Seeds::builtin()
        .apply(&["local-user".to_owned()], &app.state)
        .await
        .unwrap();
    app
}

fn budget(app: &TestApp) -> u32 {
    app.state
        .settings
        .snapshot()
        .get(&AUTH_IP_RATE_LIMIT_PER_MINUTE)
}

/// A fresh token, logged in from `peer` (not yet in the session cache).
async fn token_from(app: &TestApp, peer: &str) -> String {
    let reply = app
        .send(&from(login(SEED_USERNAME, SEED_PASSWORD), peer))
        .await
        .unwrap();
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    reply.json::<LoginResponse>().unwrap().access_token.unwrap()
}

/// A token whose session is in the cache (used once from `peer`).
async fn cached_token_from(app: &TestApp, peer: &str) -> String {
    let token = token_from(app, peer).await;
    let reply = app
        .send(&from(bearer(CURRENT_USER, &token), peer))
        .await
        .unwrap();
    assert_eq!(reply.status, 200);
    token
}

/// Random tokens from `peer` until one is refused with 429; the number of 401s before it.
async fn spray_until_gated(app: &TestApp, peer: &str, max: u32) -> u32 {
    for sent in 0..=max {
        let reply = app
            .send(&from(bearer(CURRENT_USER, RANDOM_TOKEN), peer))
            .await
            .unwrap();
        match reply.status {
            401 => continue,
            429 => {
                assert_eq!(error_of(&reply), "Please try later");
                assert!(reply.headers.contains_key("retry-after"));
                return sent;
            }
            other => panic!("unexpected status {other}"),
        }
    }
    panic!("never gated after {max} requests");
}

async fn current_user(app: &TestApp, token: &str, peer: &str) -> u16 {
    app.send(&from(bearer(CURRENT_USER, token), peer))
        .await
        .unwrap()
        .status
}

#[tokio::test]
async fn bearer_failures_spend_the_shared_budget_and_then_gate_uncached_tokens() {
    let app = seeded(TestApp::start().await.unwrap()).await;
    let uncached = token_from(&app, OTHER_PEER).await;

    let rejected = spray_until_gated(&app, DEFAULT, budget(&app)).await;
    let gated = app.send(&bearer(CURRENT_USER, RANDOM_TOKEN)).await.unwrap();
    let login_here = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();

    assert_eq!(
        rejected,
        budget(&app),
        "one budget token per rejected token"
    );
    assert_eq!(gated.status, 429);
    assert_eq!(login_here.status, 429, "logins share the per-IP budget");
    // A cache miss needs a lookup: refused before it, even for a valid token ...
    assert_eq!(current_user(&app, &uncached, DEFAULT).await, 429);
    // ... which works from anywhere else.
    assert_eq!(current_user(&app, &uncached, OTHER_PEER).await, 200);
}

#[tokio::test]
async fn bearer_spraying_does_not_block_a_cached_valid_token_from_the_same_ip() {
    let app = seeded(TestApp::start().await.unwrap()).await;
    let cached = cached_token_from(&app, DEFAULT).await;

    spray_until_gated(&app, DEFAULT, budget(&app)).await;

    for _ in 0..3 {
        assert_eq!(current_user(&app, &cached, DEFAULT).await, 200);
    }
}

#[tokio::test]
async fn logout_with_a_cached_token_works_while_the_ip_is_gated() {
    let app = seeded(TestApp::start().await.unwrap()).await;
    let cached = cached_token_from(&app, DEFAULT).await;
    spray_until_gated(&app, DEFAULT, budget(&app)).await;

    let logout = app.send(&bearer("/api/logout", &cached)).await.unwrap();

    assert_eq!((logout.status, logout.body.len()), (200, 0));
    assert_eq!(
        current_user(&app, &cached, OTHER_PEER).await,
        401,
        "the session is revoked server-side"
    );
}

#[tokio::test]
async fn login_denials_from_an_ip_do_not_gate_bearer_requests_from_it() {
    let app = seeded(TestApp::start().await.unwrap()).await;
    let uncached = token_from(&app, OTHER_PEER).await;
    // Distinct usernames: the per-IP limit, not an account lockout, refuses the last one.
    let mut statuses = Vec::new();
    for i in 0..=budget(&app) {
        let name = format!("user{i}");
        let reply = app.send(&login(&name, "wrong-password")).await.unwrap();
        statuses.push(reply.status);
    }

    assert_eq!(statuses.last(), Some(&429), "{statuses:?}");
    assert_eq!(
        current_user(&app, &uncached, DEFAULT).await,
        200,
        "a valid token's lookup is not gated by login denials"
    );
}

#[tokio::test]
async fn fallback_bucket_spraying_does_not_block_cached_tokens_of_other_proxied_clients() {
    let options = TestOptions {
        trusted_proxies: vec!["10.0.0.0/8".parse().unwrap()],
        ..TestOptions::default()
    };
    let app = seeded(TestApp::start_with_options(options).await.unwrap()).await;
    let cached = cached_token_from(&app, PROXY_B).await;
    let uncached = token_from(&app, PROXY_B).await;

    // Both proxies' clients share the one fallback bucket.
    spray_until_gated(&app, PROXY_A, budget(&app) * FALLBACK_FACTOR).await;

    assert_eq!(current_user(&app, &cached, PROXY_B).await, 200);
    assert_eq!(current_user(&app, &uncached, PROXY_B).await, 429);
    assert_eq!(current_user(&app, &uncached, DEFAULT).await, 200);
}

#[tokio::test]
async fn valid_bearer_requests_do_not_spend_the_per_ip_budget() {
    let app = seeded(TestApp::start().await.unwrap()).await;
    let token = token_from(&app, DEFAULT).await;

    for _ in 0..budget(&app) * 2 {
        assert_eq!(current_user(&app, &token, DEFAULT).await, 200);
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
    spray_until_gated(&app, DEFAULT, budget(&app)).await;

    let options = app
        .send(&Request::new("GET", "/api/login-options"))
        .await
        .unwrap();

    assert_eq!(options.status, 200);
    assert!(options.json::<LoginOptions>().unwrap().is_empty());
}
