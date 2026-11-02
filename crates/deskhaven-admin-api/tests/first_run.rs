//! First-run setup through the real router, and the M3-02 acceptance flow (first run ->
//! create the owner -> log in -> enrol TOTP -> log out -> the next login needs TOTP; the
//! setup token works once). Browser E2E (Playwright) for the same flow is M3-15.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::login::{console_of, set_cookies};
use common::{CSRF_HEADER_NAME, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::console_audit::ActorKind;
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::users::Role;
use deskhaven_db::entity::console_audit;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

const SETUP: &str = "/admin/api/v1/setup";
const OWNER_PASSWORD: &str = "first owner password 2026";

fn setup_body(token: &str) -> Value {
    json!({
        "token": token,
        "username": "root",
        "display_name": "The Owner",
        "email": "root@example.com",
        "password": OWNER_PASSWORD,
    })
}

#[tokio::test]
async fn the_status_says_whether_the_owner_is_still_needed() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let b = db.backend;

        let before = h.send(Method::GET, SETUP, None, None, None).await;
        h.user("existing", Role::Owner).await;
        let after = h.send(Method::GET, SETUP, None, None, None).await;

        assert_eq!(before.status, 200, "{b}");
        assert_eq!(
            before.body,
            json!({ "required": true, "public_url": null }),
            "{b}"
        );
        assert_eq!(after.body["required"], false, "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_token_creates_the_owner_once_signs_them_in_and_is_audited_by_them() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let token = h.state.setup.issue().await.unwrap();
        let b = db.backend;

        let done = h.post(SETUP, None, setup_body(token.expose())).await;
        let reused = h.post(SETUP, None, setup_body(token.expose())).await;

        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["status"], "signed_in", "{b}");
        assert_eq!(done.body["user"]["role"], "owner", "{b}");
        assert_eq!(done.body["user"]["display_name"], "The Owner", "{b}");
        let owner = console_of(&done);
        assert_eq!(h.get("/admin/api/v1/me", &owner).await.status, 200, "{b}");
        assert_eq!(reused.status, 409, "{b}");
        assert_eq!(reused.code(), "setup_completed", "{b}");
        let rows = console_audit::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(
            rows.len(),
            1,
            "{b}: written with the owner, not by the fallback"
        );
        let row = &rows[0];
        assert_eq!(row.action, "setup.complete", "{b}");
        assert_eq!(row.actor_kind, ActorKind::User, "{b}");
        assert_eq!(row.actor_id, done.body["user"]["id"].as_i64(), "{b}");
        assert!(row.after.as_deref().unwrap().contains("\"root\""), "{b}");
        assert!(
            !row.after.as_deref().unwrap().contains(OWNER_PASSWORD),
            "{b}: no password in the audit row"
        );
    })
    .await;
}

#[tokio::test]
async fn bad_tokens_weak_passwords_and_plain_http_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let token = h.state.setup.issue().await.unwrap();
        let b = db.backend;

        let unknown = h.post(SETUP, None, setup_body(&"A".repeat(43))).await;
        let mut weak = setup_body(token.expose());
        weak["password"] = json!("short");
        let weak = h.post(SETUP, None, weak).await;
        let plain = h
            .raw(common::request(
                Method::POST,
                SETUP,
                &[
                    ("origin", "http://192.0.2.10:21114"),
                    ("host", "192.0.2.10:21114"),
                    (CSRF_HEADER_NAME, "anonymous"),
                ],
                Some(setup_body(token.expose())),
            ))
            .await;
        let ok = h.post(SETUP, None, setup_body(token.expose())).await;

        assert_eq!(unknown.status, 403, "{b}: {:?}", unknown.body);
        assert_eq!(unknown.code(), "invalid_setup_token", "{b}");
        assert_eq!(weak.status, 422, "{b}");
        assert!(
            weak.body["error"]["details"]["fields"]["password"].is_array(),
            "{b}"
        );
        assert_eq!(plain.code(), "insecure_transport", "{b}");
        assert_eq!(ok.status, 200, "{b}: the token survived the refusals");
    })
    .await;
}

#[tokio::test]
async fn setup_attempts_are_limited_per_address() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.set_setting("auth.ip_rate_limit.per_minute", json!(2))
            .await;
        let b = db.backend;

        let mut statuses = Vec::new();
        for _ in 0..3 {
            let reply = h.post(SETUP, None, setup_body(&"A".repeat(43))).await;
            statuses.push(reply.status.as_u16());
        }

        assert_eq!(statuses, [403, 403, 429], "{b}");
    })
    .await;
}

/// The acceptance flow of M3-02, end to end on the real router.
#[tokio::test]
async fn first_run_owner_login_totp_logout_and_login_with_totp() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let b = db.backend;
        let token = h.state.setup.issue().await.unwrap();

        // First run: create the owner.
        let created = h.post(SETUP, None, setup_body(token.expose())).await;
        assert_eq!(created.status, 200, "{b}: {:?}", created.body);
        let reuse = h.post(SETUP, None, setup_body(token.expose())).await;
        assert_eq!(reuse.code(), "setup_completed", "{b}: the token works once");

        // Log in with the password: no 2FA yet.
        let login = h.login("root", OWNER_PASSWORD).await;
        assert_eq!(login.body["status"], "signed_in", "{b}");
        let console = console_of(&login);

        // Enrol TOTP.
        let begun = h
            .post("/admin/api/v1/me/totp/enrolment", Some(&console), json!({}))
            .await;
        let secret = TotpSecret::from_base32(begun.body["secret"].as_str().unwrap()).unwrap();
        let code = totp::code_at(&secret, SystemClock.now_ms());
        let confirmed = h
            .post(
                "/admin/api/v1/me/totp/enrolment/confirm",
                Some(&console),
                json!({ "code": code }),
            )
            .await;
        assert_eq!(confirmed.status, 200, "{b}: {:?}", confirmed.body);
        let console = console_of(&confirmed);

        // Log out.
        let out = h
            .post("/admin/api/v1/auth/logout", Some(&console), json!({}))
            .await;
        assert_eq!(out.status, 204, "{b}");
        assert_eq!(h.get("/admin/api/v1/me", &console).await.status, 401, "{b}");

        // The next login needs TOTP, and no cookie comes before it.
        let step1 = h.login("root", OWNER_PASSWORD).await;
        assert_eq!(step1.body["status"], "totp_required", "{b}");
        assert!(set_cookies(&step1).is_empty(), "{b}");
        let next = totp::code_at_step(&secret, totp::step_at(SystemClock.now_ms()) + 1);
        let step2 = h
            .post(
                "/admin/api/v1/auth/login/totp",
                None,
                json!({
                    "username": "root",
                    "challenge": step1.body["challenge"],
                    "code": next,
                }),
            )
            .await;
        assert_eq!(step2.status, 200, "{b}: {:?}", step2.body);
        let me = h.get("/admin/api/v1/me", &console_of(&step2)).await;
        assert_eq!(me.body["username"], "root", "{b}");
        assert_eq!(me.body["totp_enabled"], true, "{b}");
    })
    .await;
}
