//! "My account" (M3-13), turning TOTP off: the current password AND a current TOTP or
//! recovery code (a stolen password plus session cannot remove the factor), refused while
//! `auth.tfa.enforce` covers the user. Wrong codes count against the login throttle. An
//! OIDC account has no password: the code alone. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::own::{ACCOUNT, DISABLE, ME, external_console};
use common::{PASSWORD, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::entity::console_audit;
use deskhaven_db::entity::user::AuthSource;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;

async fn audit_rows(db: &TestDb, action: &str) -> Vec<console_audit::Model> {
    console_audit::Entity::find()
        .filter(console_audit::Column::Action.eq(action))
        .all(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn turning_totp_off_needs_the_password_and_a_code_and_a_failure_spends_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let enrolled = h.enrol_through_console(&alice).await;
        let alice = enrolled.console.clone();
        let code = enrolled.next_code();
        let b = db.backend;
        let off = |body: serde_json::Value| h.post(DISABLE, Some(&alice), body);

        let wrong_password = off(json!({ "current_password": "nope", "code": code })).await;
        let no_code = off(json!({ "current_password": PASSWORD })).await;
        let no_password = off(json!({ "code": code })).await;
        let wrong_code = off(json!({ "current_password": PASSWORD, "code": "000000" })).await;
        let still = h.get(ACCOUNT, &alice).await.body["totp_enabled"].clone();
        // The failures above did not spend the code.
        let done = off(json!({ "current_password": PASSWORD, "code": code })).await;

        assert_eq!(wrong_password.status, 422, "{b}: never 401");
        assert_eq!(wrong_password.code(), "wrong_password", "{b}");
        assert_eq!(
            no_code.code(),
            "validation_failed",
            "{b}: {:?}",
            no_code.body
        );
        assert_eq!(no_password.code(), "validation_failed", "{b}");
        assert_eq!(wrong_code.status, 422, "{b}");
        assert_eq!(wrong_code.code(), "invalid_code", "{b}");
        assert_eq!(still, true, "{b}");
        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["totp_enabled"], false, "{b}");
        let rows = audit_rows(db, "user.totp_disable").await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].actor_id, Some(alice_id), "{b}");
        assert_eq!(h.get(ME, &alice).await.status, 200, "{b}: session kept");
        let login = h.login("alice", PASSWORD).await;
        assert_eq!(login.body["status"], "signed_in", "{b}");
        let again = off(json!({ "current_password": PASSWORD, "code": "123456" })).await;
        assert_eq!(again.code(), "conflict", "{b}: already off");
    })
    .await;
}

#[tokio::test]
async fn a_recovery_code_turns_totp_off_once() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let enrolled = h.enrol_through_console(&alice).await;
        let b = db.backend;
        let body = json!({ "current_password": PASSWORD, "code": enrolled.recovery[0] });

        let done = h.post(DISABLE, Some(&enrolled.console), body).await;

        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["totp_enabled"], false, "{b}");
        let account = h.get(ACCOUNT, &enrolled.console).await;
        assert_eq!(
            account.body["recovery_codes_left"], 0,
            "{b}: all codes are gone"
        );
    })
    .await;
}

#[tokio::test]
async fn wrong_codes_count_against_the_login_throttle() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let enrolled = h.enrol_through_console(&alice).await;
        let b = db.backend;
        let mut replies = Vec::new();
        for _ in 0..5 {
            let body = json!({ "current_password": PASSWORD, "code": "000000" });
            replies.push(h.post(DISABLE, Some(&enrolled.console), body).await);
        }
        let locked = h.login("alice", PASSWORD).await;
        let right = json!({ "current_password": PASSWORD, "code": enrolled.next_code() });
        let refused = h.post(DISABLE, Some(&enrolled.console), right).await;

        for reply in &replies[..4] {
            assert_eq!(reply.code(), "invalid_code", "{b}");
        }
        assert_eq!(locked.status, 429, "{b}: {:?}", locked.body);
        assert_eq!(refused.status, 429, "{b}: even the right code waits");
        let account = h.get(ACCOUNT, &enrolled.console).await;
        assert_eq!(account.body["totp_enabled"], true, "{b}");
    })
    .await;
}

#[tokio::test]
async fn enforcement_blocks_turning_totp_off_before_anything_is_checked() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("adam", Role::Admin).await;
        h.user("alice", Role::User).await;
        let adam = h.signed_in("adam").await;
        let adam = h.enrol_through_console(&adam).await;
        let alice = h.signed_in("alice").await;
        let alice = h.enrol_through_console(&alice).await;
        h.set_setting("auth.tfa.enforce", json!("admins")).await;
        let b = db.backend;
        let adam_body = json!({ "current_password": PASSWORD, "code": adam.next_code() });
        let alice_body = json!({ "current_password": PASSWORD, "code": alice.next_code() });

        let admin = h
            .post(DISABLE, Some(&adam.console), adam_body.clone())
            .await;
        let still = h.get(ACCOUNT, &adam.console).await;
        // `admins` does not cover a plain user.
        let user = h.post(DISABLE, Some(&alice.console), alice_body).await;

        assert_eq!(admin.status, 409, "{b}: {:?}", admin.body);
        assert_eq!(admin.code(), "tfa_enforced", "{b}");
        assert_eq!(still.body["totp_enabled"], true, "{b}");
        assert_eq!(still.body["tfa_required"], true, "{b}");
        assert_eq!(user.status, 200, "{b}: {:?}", user.body);
        assert_eq!(audit_rows(db, "user.totp_disable").await.len(), 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn an_oidc_account_with_totp_turns_it_off_with_the_code_alone() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let oidc = external_console(&h, "oscar", AuthSource::Oidc).await;
        let b = db.backend;

        // Nothing to turn off: an OIDC sign-in never holds a local second factor.
        let none = h
            .post(DISABLE, Some(&oidc), json!({ "code": "123456" }))
            .await;
        let totp = h.enrol(oidc.user_id).await;
        let wrong = h
            .post(DISABLE, Some(&oidc), json!({ "code": "000000" }))
            .await;
        let done = h
            .post(DISABLE, Some(&oidc), json!({ "code": totp.next_code() }))
            .await;

        assert_eq!(none.code(), "not_local_account", "{b}");
        assert_eq!(wrong.code(), "invalid_code", "{b}");
        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["totp_enabled"], false, "{b}");
        assert_eq!(audit_rows(db, "user.totp_disable").await.len(), 1, "{b}");
    })
    .await;
}
