//! Console OIDC login through the real router against the mock IdP (M3-02): providers,
//! start (binding cookie), completion only in the browser that started, failures as
//! `oidc_failed` reasons, login events, and no TOTP restriction for OIDC accounts.
//! Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;
#[path = "../../deskhaven-core/tests/common/oidc_idp.rs"]
mod oidc_idp;

use axum::http::Method;
use common::Harness;
use common::login::{console_of, cookie_value, harness_with};
use deskhaven_admin_api::{AdminRoutes, OIDC_BINDING_COOKIE};
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::oidc::NewProvider;
use deskhaven_core::state::StateConfig;
use deskhaven_db::entity::login_event;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use oidc_idp::{AUTH_CODE, AuthRequest, CLIENT_ID, CLIENT_SECRET, MockIdp, TokenSpec};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

const START: &str = "/admin/api/v1/auth/oidc/start";
const COMPLETE: &str = "/admin/api/v1/auth/oidc/complete";

struct Env {
    h: Harness,
    idp: MockIdp,
}

async fn env(db: &TestDb) -> Env {
    let config = StateConfig {
        public_url: "https://rd.example.com".to_owned(),
        console_url: String::new(),
    };
    let h = harness_with(db, AdminRoutes::new(), &config).await;
    let idp = MockIdp::start().await;
    let mut new = NewProvider::new("corp", idp.issuer.clone(), CLIENT_ID);
    new.client_secret = Some(CLIENT_SECRET.to_owned());
    new.auto_provision = true;
    h.state.oidc.create_provider(new).await.unwrap();
    Env { h, idp }
}

impl Env {
    /// Starts a flow; the IdP request and the binding cookie (`name=value`).
    async fn start(&self) -> (AuthRequest, String) {
        let reply = self
            .h
            .post(START, None, json!({ "provider": "corp" }))
            .await;
        assert_eq!(reply.status, 200, "{:?}", reply.body);
        let url = url::Url::parse(reply.body["url"].as_str().unwrap()).unwrap();
        let binding = cookie_value(&reply, OIDC_BINDING_COOKIE).unwrap();
        (
            AuthRequest::parse(&url),
            format!("{OIDC_BINDING_COOKIE}={binding}"),
        )
    }

    async fn complete(&self, body: Value, cookie: Option<&str>) -> common::Reply {
        let extra: Vec<(&str, &str)> = cookie.map(|c| ("cookie", c)).into_iter().collect();
        self.h.post_with(COMPLETE, None, body, &extra).await
    }

    async fn mount_token(&self, request: &AuthRequest) {
        let spec = TokenSpec::for_flow("sub-1", &request.nonce, SystemClock.now_ms());
        self.idp.mount_token(&spec).await;
    }
}

fn answer(request: &AuthRequest) -> Value {
    json!({ "state": request.state, "code": AUTH_CODE })
}

#[tokio::test]
async fn providers_are_listed_and_unknown_ones_cannot_start() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let b = db.backend;

        let list = env
            .h
            .send(
                Method::GET,
                "/admin/api/v1/auth/oidc/providers",
                None,
                None,
                None,
            )
            .await;
        let unknown = env.h.post(START, None, json!({ "provider": "nope" })).await;

        assert_eq!(list.body, json!([{ "name": "corp" }]), "{b}");
        assert_eq!(unknown.status, 404, "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_starting_browser_completes_the_login_into_a_fresh_session() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let (request, binding) = env.start().await;
        env.mount_token(&request).await;
        let b = db.backend;

        let reply = env.complete(answer(&request), Some(&binding)).await;

        assert_eq!(reply.status, 200, "{b}: {:?}", reply.body);
        assert_eq!(reply.body["status"], "signed_in", "{b}");
        assert_eq!(
            cookie_value(&reply, OIDC_BINDING_COOKIE).as_deref(),
            Some(""),
            "{b}"
        );
        let console = console_of(&reply);
        assert_eq!(
            env.h.get("/admin/api/v1/me", &console).await.status,
            200,
            "{b}"
        );
        let events = login_event::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(events.len(), 1, "{b}");
        assert_eq!(events[0].method, login_event::Method::Oidc, "{b}");
        assert_eq!(events[0].client, login_event::Client::Console, "{b}");
        assert_eq!(events[0].result, "success", "{b}");
        assert_eq!(events[0].username_attempted, "corp:sub-1", "{b}");
    })
    .await;
}

#[tokio::test]
async fn another_browser_cannot_complete_and_the_flow_survives() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let (request, binding) = env.start().await;
        let (_, other_binding) = env.start().await;
        env.mount_token(&request).await;
        let b = db.backend;

        let without = env.complete(answer(&request), None).await;
        let other = env.complete(answer(&request), Some(&other_binding)).await;
        let owner = env.complete(answer(&request), Some(&binding)).await;

        for reply in [&without, &other] {
            assert_eq!(reply.status, 403, "{b}");
            assert_eq!(reply.code(), "oidc_failed", "{b}");
            assert_eq!(reply.body["error"]["details"]["reason"], "expired", "{b}");
        }
        assert!(env.idp.token_requests().await.len() <= 1, "{b}");
        assert_eq!(owner.status, 200, "{b}: {:?}", owner.body);
        let replay = env.complete(answer(&request), Some(&binding)).await;
        assert_eq!(
            replay.body["error"]["details"]["reason"], "expired",
            "{b}: single use"
        );
    })
    .await;
}

#[tokio::test]
async fn a_denied_login_reports_its_reason() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        let (request, binding) = env.start().await;
        let b = db.backend;

        let body = json!({ "state": request.state, "error": "access_denied" });
        let reply = env.complete(body, Some(&binding)).await;

        assert_eq!(reply.code(), "oidc_failed", "{b}");
        assert_eq!(reply.body["error"]["details"]["reason"], "denied", "{b}");
        let events = login_event::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(events[0].result, "oidc_denied", "{b}");
    })
    .await;
}

#[tokio::test]
async fn enforced_totp_does_not_restrict_an_oidc_account() {
    each_migrated_backend(async |db: &TestDb| {
        let env = env(db).await;
        env.h.set_setting("auth.tfa.enforce", json!("all")).await;
        let (request, binding) = env.start().await;
        env.mount_token(&request).await;
        let b = db.backend;

        let reply = env.complete(answer(&request), Some(&binding)).await;

        assert_eq!(reply.status, 200, "{b}: {:?}", reply.body);
        assert_eq!(reply.body["user"]["setup_required"], Value::Null, "{b}");
    })
    .await;
}
