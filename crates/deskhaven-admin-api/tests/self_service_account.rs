//! "My account" (M3-13), credentials and profile: the account view, the profile edit (the
//! display name only: the email is not editable here), recovery code regeneration
//! (re-authenticates), and the acceptance flow: enrol through the console endpoints, then
//! the client's `/api/login` asks for the code. TOTP disable is in `self_service_totp`.
//! Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::own::external_console;
use common::own::{
    ACCOUNT, DISABLE, ME, RECOVERY, client_code_step, client_login, client_password_step,
};
use common::{PASSWORD, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::tfa::totp;
use deskhaven_core::users::{ProfileUpdate, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::entity::user::AuthSource;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

async fn audit_rows(db: &TestDb, action: &str) -> Vec<console_audit::Model> {
    console_audit::Entity::find()
        .filter(console_audit::Column::Action.eq(action))
        .all(&db.conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_account_view_states_what_the_account_may_do() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let local = h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let b = db.backend;

        let view = h.get(ACCOUNT, &alice).await;

        assert_eq!(view.status, 200, "{b}: {:?}", view.body);
        assert_eq!(view.body["id"], local, "{b}");
        assert_eq!(view.body["username"], "alice", "{b}");
        assert_eq!(view.body["auth_source"], "local", "{b}");
        assert_eq!(view.body["can_change_password"], true, "{b}");
        assert_eq!(view.body["can_use_totp"], true, "{b}");
        assert_eq!(view.body["totp_enabled"], false, "{b}");
        assert_eq!(view.body["recovery_codes_left"], 0, "{b}");
        assert_eq!(view.body["tfa_required"], false, "{b}");
        assert!(view.body.get("password_hash").is_none(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn directory_and_sso_accounts_have_no_local_password_to_change_or_codes_to_regenerate() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let ldap = external_console(&h, "lena", AuthSource::Ldap).await;
        let oidc = external_console(&h, "oscar", AuthSource::Oidc).await;
        let b = db.backend;
        let body = json!({ "current_password": "x", "code": "123456" });

        let ldap_view = h.get(ACCOUNT, &ldap).await;
        let oidc_view = h.get(ACCOUNT, &oidc).await;
        let oidc_codes = h.post(RECOVERY, Some(&oidc), body).await;

        assert_eq!(ldap_view.body["auth_source"], "ldap", "{b}");
        assert_eq!(ldap_view.body["can_change_password"], false, "{b}");
        assert_eq!(
            ldap_view.body["can_use_totp"], true,
            "{b}: LDAP accounts enrol TOTP"
        );
        assert_eq!(oidc_view.body["can_change_password"], false, "{b}");
        assert_eq!(oidc_view.body["can_use_totp"], false, "{b}: OIDC is exempt");
        assert_eq!(oidc_codes.code(), "not_local_account", "{b}");
    })
    .await;
}

#[tokio::test]
async fn the_profile_edits_the_display_name_of_the_session_user_only() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        let bob_id = h.user("bob", Role::User).await;
        let verified = ProfileUpdate {
            email: Some(Some("alice@example.test".to_owned())),
            email_verified: Some(true),
            ..ProfileUpdate::default()
        };
        h.state
            .users
            .update_profile(alice_id, verified)
            .await
            .unwrap();
        let alice = h.signed_in("alice").await;
        let b = db.backend;
        let patch = |body: Value| h.as_console(Method::PATCH, ME, &alice, Some(body));

        let renamed = patch(json!({ "display_name": "Alice A." })).await;
        // Saying the same again changes nothing: 200, and no second audit row.
        let unchanged = patch(json!({ "display_name": " Alice A. " })).await;
        let account = h.get(ACCOUNT, &alice).await;
        // The email is not editable here, and nothing else about the account is either.
        let email = patch(json!({ "email": "alice@new.example.test" })).await;
        let both = patch(json!({ "display_name": "x", "email": "" })).await;
        let self_verify = patch(json!({ "email_verified": true })).await;
        let promote = patch(json!({ "role": "owner" })).await;
        let foreign = patch(json!({ "id": bob_id, "display_name": "pwned" })).await;
        let empty = patch(json!({})).await;

        assert_eq!(renamed.status, 200, "{b}: {:?}", renamed.body);
        assert_eq!(renamed.body["display_name"], "Alice A.", "{b}");
        assert_eq!(unchanged.status, 200, "{b}: {:?}", unchanged.body);
        assert_eq!(unchanged.body["display_name"], "Alice A.", "{b}");
        assert_eq!(account.body["email"], "alice@example.test", "{b}");
        assert_eq!(account.body["email_verified"], true, "{b}");
        for refused in [&email, &both, &self_verify, &promote, &foreign, &empty] {
            assert_eq!(refused.status, 422, "{b}: {:?}", refused.body);
        }
        let bob = h.state.users.get(bob_id).await.unwrap();
        assert_eq!(bob.display_name, "bob", "{b}: another account is untouched");
        let alice_row = h.state.users.get(alice_id).await.unwrap();
        assert_eq!(alice_row.role, Role::User, "{b}");
        assert_eq!(
            alice_row.email.as_deref(),
            Some("alice@example.test"),
            "{b}"
        );
        let rows = audit_rows(db, "user.profile_update").await;
        assert_eq!(rows.len(), 1, "{b}: one per applied change");
        assert_eq!(rows[0].actor_id, Some(alice_id), "{b}");
        assert_eq!(rows[0].target_id, alice_id.to_string(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn regenerating_recovery_codes_re_authenticates_and_replaces_every_code() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let proof =
            |password: &str, code: &str| json!({ "current_password": password, "code": code });
        let early = h
            .post(RECOVERY, Some(&alice), proof(PASSWORD, "123456"))
            .await;
        let enrolled = h.enrol_through_console(&alice).await;
        let alice = enrolled.console.clone();
        let b = db.backend;
        let code = enrolled.next_code();

        let wrong_password = h.post(RECOVERY, Some(&alice), proof("nope", &code)).await;
        let wrong_code = h
            .post(RECOVERY, Some(&alice), proof(PASSWORD, "000000"))
            .await;
        let password_only = h
            .post(
                RECOVERY,
                Some(&alice),
                json!({ "current_password": PASSWORD }),
            )
            .await;
        // The failures spent nothing: the same code still works, once.
        let fresh = h.post(RECOVERY, Some(&alice), proof(PASSWORD, &code)).await;
        let replayed = h.post(RECOVERY, Some(&alice), proof(PASSWORD, &code)).await;

        assert_eq!(early.status, 409, "{b}: nothing to regenerate");
        assert_eq!(wrong_password.status, 422, "{b}");
        assert_eq!(wrong_password.code(), "wrong_password", "{b}");
        assert_eq!(
            wrong_code.code(),
            "invalid_code",
            "{b}: a password alone is not enough"
        );
        assert_eq!(password_only.code(), "validation_failed", "{b}");
        assert_eq!(
            replayed.code(),
            "invalid_code",
            "{b}: a spent step is not reusable"
        );
        assert_eq!(fresh.status, 200, "{b}: {:?}", fresh.body);
        assert_eq!(fresh.headers["cache-control"], "no-store", "{b}");
        let codes: Vec<&str> = fresh.body["recovery_codes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_str().unwrap())
            .collect();
        assert_eq!(codes.len(), 10, "{b}");
        assert!(
            codes
                .iter()
                .all(|c| !enrolled.recovery.iter().any(|o| o == c)),
            "{b}"
        );
        let left = h.get(ACCOUNT, &alice).await.body["recovery_codes_left"].clone();
        assert_eq!(left, 10, "{b}");

        // The old codes are dead at the client's second login step, the new ones work.
        let (_, challenge) = client_login(&h, client_password_step("alice", PASSWORD)).await;
        let old = client_login(&h, client_code_step(&challenge, &enrolled.recovery[0])).await;
        let new = client_login(&h, client_code_step(&challenge, codes[0])).await;
        assert_eq!(old.0, 401, "{b}: {:?}", old.1);
        assert_eq!(new.0, 200, "{b}: {:?}", new.1);

        let rows = audit_rows(db, "user.recovery_codes_regenerate").await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].actor_id, Some(alice_id), "{b}");
        let dump = format!("{rows:?}");
        assert!(
            codes.iter().all(|c| !dump.contains(c)),
            "{b}: no code in the audit row"
        );
    })
    .await;
}

#[tokio::test]
async fn a_user_who_enrols_through_the_console_is_asked_for_the_code_by_the_client() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let b = db.backend;

        let plain = client_login(&h, client_password_step("alice", PASSWORD)).await;
        let alice = h.signed_in("alice").await;
        let enrolled = h.enrol_through_console(&alice).await;
        let asked = client_login(&h, client_password_step("alice", PASSWORD)).await;
        let code = totp::code_at_step(&enrolled.secret, totp::step_at(SystemClock.now_ms()) + 1);
        let done = client_login(&h, client_code_step(&asked.1, &code)).await;

        assert_eq!(plain.1["type"], "access_token", "{b}: no factor yet");
        assert_eq!(asked.0, 200, "{b}");
        assert_eq!(asked.1["tfa_type"], "tfa_check", "{b}: {:?}", asked.1);
        assert!(asked.1.get("access_token").is_none(), "{b}");
        assert_eq!(done.0, 200, "{b}: {:?}", done.1);
        assert_eq!(done.1["type"], "access_token", "{b}");

        let off = h
            .post(
                DISABLE,
                Some(&enrolled.console),
                json!({ "current_password": PASSWORD, "code": enrolled.recovery[0] }),
            )
            .await;
        let after = client_login(&h, client_password_step("alice", PASSWORD)).await;
        assert_eq!(off.status, 200, "{b}: {:?}", off.body);
        assert_eq!(after.1["type"], "access_token", "{b}: the factor is gone");
    })
    .await;
}
