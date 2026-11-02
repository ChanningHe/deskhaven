//! Two-step TOTP login through the real router (03 §3.2), each reply decoded with the
//! client mirror types: challenge -> code -> token, wrong / replayed / recovery codes,
//! challenge expiry, attempt cap and binding, the shared lockout budget, enumeration and
//! login events.
#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_contract::app::TestApp;
use deskhaven_contract::check::Reply;
use deskhaven_contract::fixture::Body;
use deskhaven_contract::mirror::{
    ClientUserPayload, ErrorBody, LoginResponse, SecondFactorChallenge,
};
use deskhaven_contract::requests::{bearer, login, second_step};
use deskhaven_contract::seeds::{
    SEED_PASSWORD, SEED_USERNAME, SeedOutputs, Seeds, UNKNOWN_USERNAME,
};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_db::entity::login_event::{self, Method};
use sea_orm::{ConnectionTrait, EntityTrait, QueryOrder};
use serde_json::json;

const WRONG_PASSWORD: &str = "not-the-password";
const WRONG_CODE: &str = "wrong-2fa-code";
const EXPIRED: &str = "Verification expired, please log in again";

struct Env {
    app: TestApp,
    secret: TotpSecret,
    recovery: String,
}

async fn env_with(seeds: &[&str]) -> (TestApp, SeedOutputs) {
    let app = TestApp::start().await.unwrap();
    let names: Vec<String> = seeds.iter().map(|s| (*s).to_owned()).collect();
    let outputs = Seeds::builtin().apply(&names, &app.state).await.unwrap();
    (app, outputs)
}

async fn env() -> Env {
    let (app, outputs) = env_with(&["totp-user"]).await;
    Env {
        app,
        secret: TotpSecret::from_base32(&outputs["totp_secret"]).unwrap(),
        recovery: outputs["recovery_code"].clone(),
    }
}

impl Env {
    /// The authenticator's code `delta` steps from now.
    fn code(&self, delta: i64) -> String {
        totp::code_at_step(&self.secret, totp::step_at(SystemClock.now_ms()) + delta)
    }

    /// A six-digit code matching none of the steps the server accepts now.
    fn wrong_code(&self) -> String {
        let window: Vec<String> = (-2..=2).map(|d| self.code(d)).collect();
        ["000000", "111111", "222222"]
            .into_iter()
            .find(|c| !window.iter().any(|w| w == c))
            .unwrap()
            .to_owned()
    }

    /// Step 1 with the right password: the challenge.
    async fn challenge(&self) -> SecondFactorChallenge {
        let reply = self
            .app
            .send(&login(SEED_USERNAME, SEED_PASSWORD))
            .await
            .unwrap();
        assert_eq!(reply.status, 200, "{}", text(&reply));
        let raw: serde_json::Value = reply.json().unwrap();
        assert!(raw.get("access_token").is_none(), "{raw}");
        reply.json().unwrap()
    }

    async fn answer(&self, challenge: &SecondFactorChallenge, code: &str) -> Reply {
        let request = second_step(&challenge.secret, &challenge.user.name, code);
        self.app.send(&request).await.unwrap()
    }
}

fn text(reply: &Reply) -> String {
    String::from_utf8_lossy(&reply.body).into_owned()
}

fn error_of(reply: &Reply) -> (u16, String) {
    (reply.status, reply.json::<ErrorBody>().unwrap().error)
}

fn token_of(reply: &Reply) -> String {
    assert_eq!(reply.status, 200, "{}", text(reply));
    let body: LoginResponse = reply.json().unwrap();
    assert_eq!(body.kind.as_deref(), Some("access_token"));
    assert!(body.tfa_type.is_none() && body.secret.is_none());
    body.access_token.unwrap()
}

async fn events(app: &TestApp) -> Vec<(Method, String, bool)> {
    login_event::Entity::find()
        .order_by_asc(login_event::Column::Id)
        .all(&app.state.db)
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.method, e.result, e.user_id.is_some()))
        .collect()
}

#[tokio::test]
async fn challenge_then_totp_code_issues_a_working_token() {
    let env = env().await;

    let challenge = env.challenge().await;
    let reply = env.answer(&challenge, &env.code(0)).await;
    let token = token_of(&reply);
    let me = env
        .app
        .send(&bearer("/api/currentUser", &token))
        .await
        .unwrap();

    assert_eq!(
        (challenge.kind.as_str(), challenge.tfa_type.as_str()),
        ("email_check", "tfa_check")
    );
    assert_eq!(challenge.user.name, SEED_USERNAME);
    assert_eq!(challenge.secret.len(), 43);
    let user: LoginResponse = reply.json().unwrap();
    assert_eq!(user.user.unwrap().name, SEED_USERNAME);
    assert_eq!(me.status, 200);
    assert_eq!(me.json::<ClientUserPayload>().unwrap().name, SEED_USERNAME);
    assert_eq!(
        events(&env.app).await,
        [
            (Method::Password, "tfa_required".to_owned(), true),
            (Method::Totp, "success".to_owned(), true),
        ]
    );
}

#[tokio::test]
async fn a_wrong_code_keeps_the_dialog_usable() {
    let env = env().await;
    let challenge = env.challenge().await;

    let wrong = env.answer(&challenge, &env.wrong_code()).await;
    let right = env.answer(&challenge, &env.code(0)).await;

    assert_eq!(error_of(&wrong), (401, WRONG_CODE.to_owned()));
    token_of(&right);
    let recorded = events(&env.app).await;
    assert_eq!(
        recorded[1],
        (Method::Totp, "invalid_code".to_owned(), false)
    );
}

#[tokio::test]
async fn a_replayed_code_is_refused_on_a_new_challenge() {
    let env = env().await;
    let code = env.code(0);
    token_of(&env.answer(&env.challenge().await, &code).await);

    let replay = env.answer(&env.challenge().await, &code).await;

    assert_eq!(error_of(&replay), (401, WRONG_CODE.to_owned()));
}

#[tokio::test]
async fn a_challenge_is_single_use() {
    let env = env().await;
    let challenge = env.challenge().await;
    token_of(&env.answer(&challenge, &env.code(0)).await);

    let again = env.answer(&challenge, &env.code(1)).await;

    assert_eq!(error_of(&again), (401, EXPIRED.to_owned()));
}

#[tokio::test]
async fn an_expired_challenge_must_start_over() {
    let env = env().await;
    let challenge = env.challenge().await;
    // There is no clock control in the contract app: expire it behind the service's back.
    env.app
        .state
        .db
        .execute_unprepared("UPDATE auth_challenges SET expires_at = 0")
        .await
        .unwrap();

    let reply = env.answer(&challenge, &env.code(0)).await;

    assert_eq!(error_of(&reply), (401, EXPIRED.to_owned()));
    assert_eq!(
        events(&env.app).await[1],
        (Method::Totp, "invalid_challenge".to_owned(), false)
    );
}

#[tokio::test]
async fn challenge_attempts_are_capped() {
    let env = env().await;
    // Keep the account lockout out of the way: this is the challenge's own cap.
    env.app
        .set_setting("auth.lockout.max_failures", json!(50))
        .await
        .unwrap();
    let challenge = env.challenge().await;

    for _ in 0..deskhaven_core::tfa::MAX_CHALLENGE_ATTEMPTS {
        let reply = env.answer(&challenge, &env.wrong_code()).await;
        assert_eq!(error_of(&reply), (401, WRONG_CODE.to_owned()));
    }
    let right = env.answer(&challenge, &env.code(0)).await;

    assert_eq!(error_of(&right), (401, EXPIRED.to_owned()));
    // A new login still works.
    token_of(&env.answer(&env.challenge().await, &env.code(0)).await);
}

#[tokio::test]
async fn a_challenge_only_answers_its_username_and_device() {
    let env = env().await;
    let challenge = env.challenge().await;
    let mut other_device = second_step(&challenge.secret, SEED_USERNAME, &env.code(0));
    if let Body::Json(body) = &mut other_device.body {
        body["id"] = json!("987654321");
    }
    let other_user = second_step(&challenge.secret, UNKNOWN_USERNAME, &env.code(0));

    let from_device = env.app.send(&other_device).await.unwrap();
    let as_other = env.app.send(&other_user).await.unwrap();
    let right = env.answer(&challenge, &env.code(0)).await;

    assert_eq!(error_of(&from_device), (401, EXPIRED.to_owned()));
    assert_eq!(error_of(&as_other), (401, EXPIRED.to_owned()));
    token_of(&right);
}

#[tokio::test]
async fn a_recovery_code_logs_in_once() {
    let env = env().await;

    let first = env.answer(&env.challenge().await, &env.recovery).await;
    let again = env.answer(&env.challenge().await, &env.recovery).await;

    token_of(&first);
    assert_eq!(error_of(&again), (401, WRONG_CODE.to_owned()));
    let recorded = events(&env.app).await;
    assert_eq!(recorded[1], (Method::Recovery, "success".to_owned(), true));
    assert_eq!(
        recorded[3],
        (Method::Recovery, "invalid_code".to_owned(), false)
    );
}

#[tokio::test]
async fn password_ok_with_wrong_codes_reaches_the_lockout() {
    // M2-06 scenario over HTTP: default auth.lockout.max_failures = 5 counts the password
    // step and the code guesses together.
    let env = env().await;
    let challenge = env.challenge().await;
    let mut statuses = Vec::new();

    for _ in 0..5 {
        statuses.push(env.answer(&challenge, &env.wrong_code()).await.status);
    }
    let password_again = env
        .app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();

    assert_eq!(statuses, [401, 401, 401, 401, 429]);
    assert_eq!(password_again.status, 429);
    assert_eq!(error_of(&password_again).1, "Please try later");
}

#[tokio::test]
async fn second_factor_does_not_enumerate_accounts() {
    let env = env().await;

    let tfa_wrong = env
        .app
        .send(&login(SEED_USERNAME, WRONG_PASSWORD))
        .await
        .unwrap();
    let unknown = env
        .app
        .send(&login(UNKNOWN_USERNAME, WRONG_PASSWORD))
        .await
        .unwrap();

    assert_eq!(tfa_wrong.status, 401);
    assert_eq!(tfa_wrong.body, unknown.body, "byte-identical");
    assert!(!text(&tfa_wrong).contains("secret"));
}

#[tokio::test]
async fn users_without_2fa_still_log_in_in_one_step() {
    let (app, _) = env_with(&["local-user"]).await;

    let reply = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();

    token_of(&reply);
    let body: serde_json::Value = reply.json().unwrap();
    assert!(body.get("secret").is_none() && body.get("tfa_type").is_none());
}

#[tokio::test]
async fn enforcement_refuses_unenrolled_users_by_role() {
    let (app, _) = env_with(&["local-user"]).await;

    app.set_setting("auth.tfa.enforce", json!("admins"))
        .await
        .unwrap();
    let user_under_admins = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();
    app.set_setting("auth.tfa.enforce", json!("all"))
        .await
        .unwrap();
    let user_under_all = app
        .send(&login(SEED_USERNAME, SEED_PASSWORD))
        .await
        .unwrap();
    let wrong_under_all = app
        .send(&login(SEED_USERNAME, WRONG_PASSWORD))
        .await
        .unwrap();

    token_of(&user_under_admins);
    assert_eq!(user_under_all.status, 403);
    assert_eq!(
        error_of(&wrong_under_all),
        (401, "Wrong credentials".to_owned())
    );
    let recorded = events(&app).await;
    assert_eq!(
        recorded[1],
        (Method::Password, "tfa_setup_required".to_owned(), false)
    );
}
