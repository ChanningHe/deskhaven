//! OIDC login through the real router (03 §3.5–3.6) against a mock IdP, each reply decoded
//! as the Rust client decodes it (`src/hbbs_http/account.rs`): `/api/oidc/auth` -> browser
//! callback -> `/api/oidc/auth-query` polling, single hand-out, device binding, the IP limit
//! on initiation, failures, login events and the login options.
#![allow(clippy::unwrap_used, clippy::panic)]

#[path = "../../deskhaven-core/tests/common/oidc_idp.rs"]
mod oidc_idp;

use deskhaven_client_api::{OIDC_INVALID_LOGIN_MESSAGE, OIDC_PENDING_MESSAGE};
use deskhaven_contract::app::{TestApp, TestOptions};
use deskhaven_contract::check::Reply;
use deskhaven_contract::mirror::{AuthBody, ErrorBody, LoginOptions, OidcAuthUrl};
use deskhaven_contract::requests::{
    bearer, from, oidc_auth, oidc_callback, oidc_confirm, oidc_query,
};
use deskhaven_contract::seeds::{SEED_RUSTDESK_ID, SEED_UUID};
use deskhaven_contract::shapes;
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::oidc::NewProvider;
use deskhaven_core::users::Status;
use deskhaven_db::entity::login_event::{self, Client, Method};
use oidc_idp::{AUTH_CODE, AuthRequest, CLIENT_ID, CLIENT_SECRET, MockIdp, TokenSpec};
use sea_orm::{EntityTrait, QueryOrder};
use serde_json::{Value, json};

const PUBLIC_URL: &str = "https://rd.example.com";
const PROVIDER: &str = "corp";

struct Env {
    app: TestApp,
    idp: MockIdp,
}

async fn env_with(adjust: impl FnOnce(&mut NewProvider)) -> Env {
    let options = TestOptions {
        public_url: PUBLIC_URL.to_owned(),
        ..TestOptions::default()
    };
    let app = TestApp::start_with_options(options).await.unwrap();
    let idp = MockIdp::start().await;
    let mut new = NewProvider::new(PROVIDER, idp.issuer.clone(), CLIENT_ID);
    new.client_secret = Some(CLIENT_SECRET.to_owned());
    adjust(&mut new);
    app.state.oidc.create_provider(new).await.unwrap();
    Env { app, idp }
}

/// Auto-provisioning on, so any identity logs in.
async fn env() -> Env {
    env_with(|p| p.auto_provision = true).await
}

fn text(reply: &Reply) -> String {
    String::from_utf8_lossy(&reply.body).into_owned()
}

fn error_of(reply: &Reply) -> (u16, String) {
    (reply.status, reply.json::<ErrorBody>().unwrap().error)
}

fn header<'r>(reply: &'r Reply, name: &str) -> &'r str {
    reply.headers.get(name).unwrap().to_str().unwrap()
}

impl Env {
    /// `/api/oidc/auth`, as the client parses it.
    async fn start(&self) -> (OidcAuthUrl, AuthRequest) {
        let reply = self.app.send(&oidc_auth(PROVIDER)).await.unwrap();
        assert_eq!(reply.status, 200, "{}", text(&reply));
        let check = shapes::lookup("OidcAuthUrl").unwrap();
        check(&reply.body).unwrap();
        let start: OidcAuthUrl = reply.json().unwrap();
        let request = AuthRequest::parse(&start.url);
        (start, request)
    }

    /// The IdP answers with a token per `shape`; the browser follows the redirect (the
    /// confirmation page) and the user confirms. The confirmation's reply.
    async fn callback(
        &self,
        request: &AuthRequest,
        subject: &str,
        shape: impl FnOnce(TokenSpec) -> TokenSpec,
    ) -> Reply {
        let spec = TokenSpec::for_flow(subject, &request.nonce, SystemClock.now_ms());
        self.idp.mount_token(&shape(spec)).await;
        let pairs = [("code", AUTH_CODE), ("state", request.state.as_str())];
        let shown = self.app.send(&oidc_callback(&pairs)).await.unwrap();
        assert_eq!(shown.status, 200, "{}", text(&shown));
        assert!(
            text(&shown).contains("<form method=\"post\">"),
            "{}",
            text(&shown)
        );
        self.app
            .send(&from(oidc_confirm(&pairs), "198.51.100.20:4000"))
            .await
            .unwrap()
    }

    async fn poll(&self, code: &str) -> Reply {
        self.app
            .send(&oidc_query(code, SEED_RUSTDESK_ID, SEED_UUID))
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn auth_answers_a_code_and_the_idp_authorization_url() {
    let env = env().await;

    let (start, request) = env.start().await;

    assert_eq!(start.code.len(), 43);
    assert!(start.url.as_str().starts_with(&env.idp.issuer));
    assert_eq!(
        request.params["redirect_uri"],
        format!("{PUBLIC_URL}/api/oidc/callback")
    );
    assert_eq!(request.params["code_challenge_method"], "S256");
}

#[tokio::test]
async fn polling_before_the_callback_says_no_authed_oidc_is_found() {
    let env = env().await;
    let (start, _) = env.start().await;

    for _ in 0..3 {
        let reply = env.poll(&start.code).await;
        assert_eq!(error_of(&reply), (404, OIDC_PENDING_MESSAGE.to_owned()));
    }
}

#[tokio::test]
async fn a_completed_login_hands_out_an_auth_body_once() {
    let env = env().await;
    let (start, request) = env.start().await;

    let page = env
        .callback(&request, "sub-1", |s| TokenSpec {
            preferred_username: Some("bob".to_owned()),
            ..s.email("bob@example.com", Some(true))
        })
        .await;
    let first = env.poll(&start.code).await;
    let second = env.poll(&start.code).await;

    assert_eq!(page.status, 200, "{}", text(&page));
    assert!(header(&page, "content-type").starts_with("text/html"));
    assert!(header(&page, "content-security-policy").contains("default-src 'none'"));
    assert_eq!(header(&page, "referrer-policy"), "no-referrer");
    assert_eq!(header(&page, "cache-control"), "no-store");
    assert_eq!(first.status, 200, "{}", text(&first));
    shapes::lookup("AuthBody").unwrap()(&first.body).unwrap();
    let body: AuthBody = first.json().unwrap();
    assert_eq!(body.kind, "access_token");
    assert_eq!(body.user.name, "bob");
    let raw: Value = first.json().unwrap();
    assert!(raw["user"]["info"].is_object(), "{raw}");
    assert!(raw.get("tfa_type").is_none() && raw.get("secret").is_none());
    let me = env
        .app
        .send(&bearer("/api/currentUser", &body.access_token))
        .await
        .unwrap();
    assert_eq!(me.status, 200, "{}", text(&me));
    assert_eq!(
        error_of(&second),
        (401, OIDC_INVALID_LOGIN_MESSAGE.to_owned())
    );
}

#[tokio::test]
async fn another_device_cannot_collect_the_login() {
    let env = env().await;
    let (start, request) = env.start().await;
    env.callback(&request, "sub-1", |s| s).await;

    let other = env
        .app
        .send(&oidc_query(&start.code, "987654321", SEED_UUID))
        .await
        .unwrap();
    let missing = env
        .app
        .send(&deskhaven_contract::fixture::Request::new(
            "GET",
            &format!("/api/oidc/auth-query?code={}", start.code),
        ))
        .await
        .unwrap();
    let own = env.poll(&start.code).await;

    assert_eq!(
        error_of(&other),
        (401, OIDC_INVALID_LOGIN_MESSAGE.to_owned())
    );
    assert_eq!(missing.status, 400);
    assert_eq!(own.status, 200, "{}", text(&own));
}

#[tokio::test]
async fn initiation_is_limited_per_ip_but_polling_is_not() {
    let env = env().await;
    env.app
        .set_setting("auth.ip_rate_limit.per_minute", json!(2))
        .await
        .unwrap();
    let (start, _) = env.start().await;
    env.start().await;

    let limited = env.app.send(&oidc_auth(PROVIDER)).await.unwrap();
    let elsewhere = env
        .app
        .send(&from(oidc_auth(PROVIDER), "192.0.2.50:1000"))
        .await
        .unwrap();

    assert_eq!(error_of(&limited), (429, "Please try later".to_owned()));
    assert!(header(&limited, "retry-after").parse::<u64>().unwrap() >= 1);
    assert_eq!(elsewhere.status, 200);
    for _ in 0..30 {
        assert_eq!(env.poll(&start.code).await.status, 404);
    }
}

#[tokio::test]
async fn failed_flows_reach_the_client_and_the_browser_as_their_message() {
    let env = env_with(|_| {}).await;
    let dave = env
        .app
        .state
        .users
        .create(deskhaven_core::users::NewUser {
            email: Some("dave@example.com".to_owned()),
            email_verified: true,
            ..deskhaven_core::users::NewUser::new(
                "dave",
                deskhaven_core::users::Credential::Local {
                    password: deskhaven_core::users::Password::new("contract-Seed-pass-1"),
                },
            )
        })
        .await
        .unwrap();
    env.app
        .state
        .users
        .set_status(dave.id, Status::Disabled)
        .await
        .unwrap();

    let (unknown, request) = env.start().await;
    let unknown_page = env.callback(&request, "nobody", |s| s).await;
    let (disabled, request) = env.start().await;
    let disabled_page = env
        .callback(&request, "dave-sub", |s| {
            s.email("dave@example.com", Some(true))
        })
        .await;

    assert_eq!(unknown_page.status, 200);
    assert!(text(&unknown_page).contains("No account is linked"));
    assert!(text(&disabled_page).contains("Account disabled"));
    let (status, message) = error_of(&env.poll(&unknown.code).await);
    assert_eq!(status, 403);
    assert!(message.starts_with("No account is linked"), "{message}");
    assert_eq!(disabled_page.status, 200);
    assert_eq!(
        error_of(&env.poll(&disabled.code).await),
        (403, "Account disabled".to_owned())
    );
}

#[tokio::test]
async fn login_events_record_callback_failures_and_collected_logins_only() {
    let env = env_with(|_| {}).await;
    let (_, failing) = env.start().await;
    env.callback(&failing, "nobody", |s| s).await;
    let bogus = oidc_callback(&[("code", "x"), ("state", &"Z".repeat(43))]);
    assert_eq!(env.app.send(&bogus).await.unwrap().status, 200);
    env.app
        .state
        .oidc
        .update_provider(
            env.app
                .state
                .oidc
                .get_provider_by_name(PROVIDER)
                .await
                .unwrap()
                .id,
            deskhaven_core::oidc::ProviderUpdate {
                auto_provision: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (start, ok) = env.start().await;
    env.callback(&ok, "sub-ok", |s| TokenSpec {
        preferred_username: Some("olga".to_owned()),
        ..s
    })
    .await;
    assert_eq!(env.poll(&start.code).await.status, 200);

    let rows = login_event::Entity::find()
        .order_by_asc(login_event::Column::Id)
        .all(&env.app.state.db)
        .await
        .unwrap();
    let summary: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r.method,
                r.result.as_str(),
                r.user_id.is_some(),
                r.username_attempted.as_str(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (Method::Oidc, "no_linked_account", false, "corp:nobody"),
            (Method::Oidc, "success", true, "olga"),
        ]
    );
    assert!(rows.iter().all(
        |r| r.client == Client::Rustdesk && r.rustdesk_id.as_deref() == Some(SEED_RUSTDESK_ID)
    ));
    // The failure comes from the browser, the success from the polling client.
    assert_eq!(rows[0].ip.as_deref(), Some("198.51.100.20"));
    assert_eq!(rows[1].ip.as_deref(), Some("203.0.113.7"));
}

#[tokio::test]
async fn an_unknown_state_shows_the_invalid_link_page_without_echoing_the_request() {
    let env = env().await;

    let reply = env
        .app
        .send(&oidc_callback(&[
            ("state", "<script>alert(1)</script>"),
            ("code", "<b>x</b>"),
        ]))
        .await
        .unwrap();
    let empty = env
        .app
        .send(&deskhaven_contract::fixture::Request::new(
            "GET",
            "/api/oidc/callback",
        ))
        .await
        .unwrap();

    assert_eq!(reply.status, 200);
    let body = text(&reply);
    assert!(
        !body.contains("<script>") && !body.contains("<b>"),
        "{body}"
    );
    assert!(body.contains("invalid, expired or already used"));
    assert_eq!(empty.status, 200);
}

#[tokio::test]
async fn login_options_list_enabled_providers_as_the_client_parses_them() {
    let env = env().await;
    let unconfigured = TestApp::start().await.unwrap();
    unconfigured
        .state
        .oidc
        .create_provider(NewProvider::new("corp", env.idp.issuer.clone(), CLIENT_ID))
        .await
        .unwrap();
    let get = deskhaven_contract::fixture::Request::new("GET", "/api/login-options");

    let plain: LoginOptions = env.app.send(&get).await.unwrap().json().unwrap();
    env.app
        .state
        .oidc
        .create_provider(NewProvider {
            icon: Some("<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_owned()),
            ..NewProvider::new("azure", env.idp.issuer.clone(), CLIENT_ID)
        })
        .await
        .unwrap();
    let with_icon: LoginOptions = env.app.send(&get).await.unwrap().json().unwrap();
    let none: LoginOptions = unconfigured.send(&get).await.unwrap().json().unwrap();

    assert_eq!(plain, ["oidc/corp"]);
    assert_eq!(with_icon.len(), 1);
    let list: Value =
        serde_json::from_str(with_icon[0].strip_prefix("common-oidc/").unwrap()).unwrap();
    assert_eq!(list[0]["name"], "azure");
    assert_eq!(list[1], json!({"name": "corp", "icon": null}));
    assert!(none.is_empty());
}

#[tokio::test]
async fn auth_refuses_malformed_unknown_and_unconfigured_requests() {
    let env = env().await;
    let unconfigured = TestApp::start().await.unwrap();
    let mut not_json = oidc_auth(PROVIDER);
    not_json
        .headers
        .insert("content-type".to_owned(), "text/plain".to_owned());
    let mut no_op = oidc_auth(PROVIDER);
    no_op.body = deskhaven_contract::fixture::Body::Json(json!({
        "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID
    }));

    let not_json = env.app.send(&not_json).await.unwrap();
    let no_op = env.app.send(&no_op).await.unwrap();
    let unknown = env.app.send(&oidc_auth("nope")).await.unwrap();
    let off = unconfigured.send(&oidc_auth(PROVIDER)).await.unwrap();

    assert_eq!(not_json.status, 400);
    assert_eq!(no_op.status, 400);
    assert_eq!(
        error_of(&unknown),
        (404, "Unknown OIDC provider".to_owned())
    );
    assert_eq!(
        error_of(&off),
        (
            404,
            "OIDC login is not configured on this server".to_owned()
        )
    );
}

/// M1: the IdP redirect names the device and completes nothing until confirmed.
#[tokio::test]
async fn the_redirect_shows_the_device_and_completes_only_on_confirmation() {
    let env = env().await;
    let (start, request) = env.start().await;
    let spec = TokenSpec::for_flow("sub-1", &request.nonce, SystemClock.now_ms());
    env.idp.mount_token(&spec).await;
    let pairs = [("code", AUTH_CODE), ("state", request.state.as_str())];

    let shown = env.app.send(&oidc_callback(&pairs)).await.unwrap();
    let shown_again = env.app.send(&oidc_callback(&pairs)).await.unwrap();
    let before = env.poll(&start.code).await;
    let exchanges_before = env.idp.token_requests().await.len();
    let confirmed = env.app.send(&oidc_confirm(&pairs)).await.unwrap();
    let after = env.poll(&start.code).await;

    let page = text(&shown);
    assert_eq!(shown.status, 200);
    // The device as it described itself at /api/oidc/auth (`requests::device_info`).
    assert!(
        page.contains(SEED_RUSTDESK_ID) && page.contains("flow-host") && page.contains("linux"),
        "{page}"
    );
    assert!(
        page.contains("<form method=\"post\">") && !page.contains("action="),
        "{page}"
    );
    assert!(
        !page.contains(AUTH_CODE) && !page.contains(&request.state),
        "{page}"
    );
    assert!(header(&shown, "content-security-policy").contains("form-action 'self'"));
    assert_eq!(header(&shown, "referrer-policy"), "no-referrer");
    assert_eq!(text(&shown_again), page);
    assert_eq!(error_of(&before), (404, OIDC_PENDING_MESSAGE.to_owned()));
    assert_eq!(exchanges_before, 0);
    assert!(
        text(&confirmed).contains("Login successful"),
        "{}",
        text(&confirmed)
    );
    assert_eq!(after.status, 200, "{}", text(&after));
}

/// M1: a confirmation works once.
#[tokio::test]
async fn a_replayed_confirmation_is_refused() {
    let env = env().await;
    let (_, request) = env.start().await;

    let first = env.callback(&request, "sub-1", |s| s).await;
    let pairs = [("code", AUTH_CODE), ("state", request.state.as_str())];
    let replay = env.app.send(&oidc_confirm(&pairs)).await.unwrap();
    let page_after = env.app.send(&oidc_callback(&pairs)).await.unwrap();

    assert!(text(&first).contains("Login successful"));
    assert!(
        text(&replay).contains("invalid, expired or already used"),
        "{}",
        text(&replay)
    );
    assert!(text(&page_after).contains("invalid, expired or already used"));
    assert_eq!(env.idp.token_requests().await.len(), 1);
}

/// M1: the device's self-description and the request are never markup on the page.
#[tokio::test]
async fn the_confirmation_page_escapes_the_device_and_never_reflects_the_request() {
    let env = env().await;
    let mut auth = oidc_auth(PROVIDER);
    auth.body = deskhaven_contract::fixture::Body::Json(json!({
        "op": PROVIDER, "id": SEED_RUSTDESK_ID, "uuid": SEED_UUID,
        "deviceInfo": { "os": "<b>os</b>", "type": "client", "name": "<script>alert(1)</script>" },
    }));
    let reply = env.app.send(&auth).await.unwrap();
    let start: OidcAuthUrl = reply.json().unwrap();
    let request = AuthRequest::parse(&start.url);

    let pairs = [("code", "<i>evil</i>"), ("state", request.state.as_str())];
    let page = text(&env.app.send(&oidc_callback(&pairs)).await.unwrap());

    assert!(
        page.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{page}"
    );
    assert!(!page.contains("<script>") && !page.contains("<b>") && !page.contains("<i>"));
    assert!(!page.contains("evil"));
}

/// M1: an IdP error asks before telling the device to stop; the confirmation fails the flow.
#[tokio::test]
async fn an_idp_error_is_confirmed_before_the_device_stops_waiting() {
    let env = env().await;
    let (start, request) = env.start().await;
    let pairs = [
        ("error", "access_denied"),
        ("state", request.state.as_str()),
    ];

    let shown = text(&env.app.send(&oidc_callback(&pairs)).await.unwrap());
    let pending = env.poll(&start.code).await;
    env.app.send(&oidc_confirm(&pairs)).await.unwrap();
    let stopped = env.poll(&start.code).await;

    assert!(
        shown.contains("Stop waiting") && !shown.contains("access_denied"),
        "{shown}"
    );
    assert_eq!(error_of(&pending), (404, OIDC_PENDING_MESSAGE.to_owned()));
    assert_eq!(
        error_of(&stopped),
        (403, "OIDC login was cancelled or denied".to_owned())
    );
}
