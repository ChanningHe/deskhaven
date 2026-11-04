//! "My account" (M3-13), credentials and profile: the account view, the profile edit
//! (an email change drops `email_verified`; a user cannot verify their own address), TOTP
//! disable and recovery code regeneration (both re-authenticate; the first is refused when
//! `auth.tfa.enforce` covers the user), and the acceptance flow: enrol through the console
//! endpoints, then the client's `/api/login` asks for the code. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::own::{
    ACCOUNT, DISABLE, ME, RECOVERY, client_code_step, client_login, client_password_step,
};
use common::{Console, Harness, PASSWORD, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::tfa::totp;
use deskhaven_core::users::{ProfileUpdate, Role};
use deskhaven_db::entity::console_audit;
use deskhaven_db::entity::user::{self, AuthSource};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

async fn audit_rows(db: &TestDb, action: &str) -> Vec<console_audit::Model> {
    console_audit::Entity::find()
        .filter(console_audit::Column::Action.eq(action))
        .all(&db.conn)
        .await
        .unwrap()
}

/// Signs `username` in with its password, then turns the account into a `source` account
/// (no local password) and drops the cached session principal.
async fn external_console(h: &Harness, username: &str, source: AuthSource) -> Console {
    let id = h.user(username, Role::User).await;
    let console = h.signed_in(username).await;
    let row = user::ActiveModel {
        id: Set(id),
        auth_source: Set(source),
        password_hash: Set(None),
        ..Default::default()
    };
    row.update(&h.state.db).await.unwrap();
    h.state.sessions.cache().flush();
    console
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
async fn directory_and_sso_accounts_have_no_local_password() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let ldap = external_console(&h, "lena", AuthSource::Ldap).await;
        let oidc = external_console(&h, "oscar", AuthSource::Oidc).await;
        let b = db.backend;
        let body = json!({ "current_password": "x" });

        let ldap_view = h.get(ACCOUNT, &ldap).await;
        let oidc_view = h.get(ACCOUNT, &oidc).await;
        let ldap_off = h.post(DISABLE, Some(&ldap), body.clone()).await;
        let oidc_off = h.post(DISABLE, Some(&oidc), body.clone()).await;
        let oidc_codes = h.post(RECOVERY, Some(&oidc), body).await;

        assert_eq!(ldap_view.body["auth_source"], "ldap", "{b}");
        assert_eq!(ldap_view.body["can_change_password"], false, "{b}");
        assert_eq!(
            ldap_view.body["can_use_totp"], true,
            "{b}: LDAP accounts enrol TOTP"
        );
        assert_eq!(oidc_view.body["can_change_password"], false, "{b}");
        assert_eq!(oidc_view.body["can_use_totp"], false, "{b}: OIDC is exempt");
        assert_eq!(ldap_off.code(), "conflict", "{b}: nothing to disable");
        assert_eq!(oidc_off.code(), "not_local_account", "{b}");
        assert_eq!(oidc_codes.code(), "not_local_account", "{b}");
    })
    .await;
}

#[tokio::test]
async fn editing_the_profile_changes_the_session_users_row_and_never_verifies_the_email() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        let bob_id = h.user("bob", Role::User).await;
        let verified = |email: &str| ProfileUpdate {
            email: Some(Some(email.to_owned())),
            email_verified: Some(true),
            ..ProfileUpdate::default()
        };
        h.state
            .users
            .update_profile(alice_id, verified("alice@example.test"))
            .await
            .unwrap();
        h.state
            .users
            .update_profile(bob_id, verified("bob@example.test"))
            .await
            .unwrap();
        let alice = h.signed_in("alice").await;
        let b = db.backend;
        let patch = |body: Value| h.as_console(Method::PATCH, ME, &alice, Some(body));

        let renamed = patch(json!({ "display_name": "Alice A." })).await;
        let same_address = patch(json!({ "email": "ALICE@example.test" })).await;
        let verified_after_rename = h.get(ACCOUNT, &alice).await.body["email_verified"].clone();
        let moved = patch(json!({ "email": "alice@new.example.test" })).await;
        let account = h.get(ACCOUNT, &alice).await;
        let taken = patch(json!({ "email": "bob@example.test" })).await;
        let invalid = patch(json!({ "email": "not an address" })).await;
        let self_verify = patch(json!({ "email_verified": true })).await;
        let promote = patch(json!({ "role": "owner" })).await;
        let foreign = patch(json!({ "id": bob_id, "display_name": "pwned" })).await;
        let empty = patch(json!({})).await;
        let removed = patch(json!({ "email": "" })).await;

        assert_eq!(renamed.status, 200, "{b}: {:?}", renamed.body);
        assert_eq!(renamed.body["display_name"], "Alice A.", "{b}");
        assert_eq!(same_address.status, 200, "{b}");
        assert_eq!(verified_after_rename, true, "{b}");
        assert_eq!(moved.status, 200, "{b}");
        assert_eq!(account.body["email"], "alice@new.example.test", "{b}");
        assert_eq!(account.body["email_verified"], false, "{b}: new address");
        assert_eq!(taken.status, 409, "{b}: {:?}", taken.body);
        assert_eq!(invalid.status, 422, "{b}");
        for refused in [&self_verify, &promote, &foreign] {
            assert_eq!(refused.status, 422, "{b}: {:?}", refused.body);
        }
        assert_eq!(empty.status, 422, "{b}");
        assert_eq!(removed.status, 200, "{b}");
        assert_eq!(removed.body["email"], Value::Null, "{b}");
        let bob = h.state.users.get(bob_id).await.unwrap();
        assert_eq!(bob.display_name, "bob", "{b}: another account is untouched");
        assert!(bob.email_verified, "{b}");
        let alice_row = h.state.users.get(alice_id).await.unwrap();
        assert_eq!(alice_row.role, Role::User, "{b}");
        let rows = audit_rows(db, "user.profile_update").await;
        assert_eq!(rows.len(), 4, "{b}: one per applied change");
        assert!(rows.iter().all(|r| r.actor_id == Some(alice_id)), "{b}");
        assert!(
            rows.iter().all(|r| r.target_id == alice_id.to_string()),
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn disabling_totp_re_authenticates_and_is_refused_when_enforced() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        h.user("adam", Role::Admin).await;
        let alice = h.signed_in("alice").await;
        let enrolled = h.enrol_through_console(&alice).await;
        let alice = enrolled.console;
        let b = db.backend;
        let again = |password: &str| {
            h.post(
                DISABLE,
                Some(&alice),
                json!({ "current_password": password }),
            )
        };

        let wrong = again("nope").await;
        assert_eq!(wrong.status, 422, "{b}: never 401");
        assert_eq!(wrong.code(), "wrong_password", "{b}");
        assert_eq!(
            h.get(ACCOUNT, &alice).await.body["totp_enabled"],
            true,
            "{b}"
        );

        // `admins` does not cover a plain user.
        h.set_setting("auth.tfa.enforce", json!("admins")).await;
        let done = again(PASSWORD).await;
        assert_eq!(done.status, 200, "{b}: {:?}", done.body);
        assert_eq!(done.body["totp_enabled"], false, "{b}");
        let rows = audit_rows(db, "user.totp_disable").await;
        assert_eq!(rows.len(), 1, "{b}");
        assert_eq!(rows[0].actor_id, Some(alice_id), "{b}");
        assert_eq!(h.get(ME, &alice).await.status, 200, "{b}: session kept");
        assert_eq!(
            h.login("alice", PASSWORD).await.body["status"],
            "signed_in",
            "{b}"
        );

        let not_enabled = again(PASSWORD).await;
        assert_eq!(not_enabled.status, 409, "{b}");
        assert_eq!(not_enabled.code(), "conflict", "{b}");
    })
    .await;
}

#[tokio::test]
async fn enforcement_blocks_disabling_before_the_password_is_even_checked() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("adam", Role::Admin).await;
        h.user("alice", Role::User).await;
        let adam = h.signed_in("adam").await;
        let adam = h.enrol_through_console(&adam).await.console;
        let alice = h.signed_in("alice").await;
        let alice = h.enrol_through_console(&alice).await.console;
        h.set_setting("auth.tfa.enforce", json!("admins")).await;
        let b = db.backend;
        let body = json!({ "current_password": PASSWORD });

        let admin = h.post(DISABLE, Some(&adam), body.clone()).await;
        let still = h.get(ACCOUNT, &adam).await;
        h.set_setting("auth.tfa.enforce", json!("all")).await;
        let user = h.post(DISABLE, Some(&alice), body).await;
        let required = h.get(ACCOUNT, &alice).await;

        assert_eq!(admin.status, 409, "{b}: {:?}", admin.body);
        assert_eq!(admin.code(), "tfa_enforced", "{b}");
        assert_eq!(still.body["totp_enabled"], true, "{b}");
        assert_eq!(still.body["tfa_required"], true, "{b}");
        assert_eq!(user.code(), "tfa_enforced", "{b}");
        assert_eq!(required.body["tfa_required"], true, "{b}");
        assert_eq!(required.body["totp_enabled"], true, "{b}");
        assert!(audit_rows(db, "user.totp_disable").await.is_empty(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn regenerating_recovery_codes_re_authenticates_and_replaces_every_code() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        let alice = h.signed_in("alice").await;
        let early = h
            .post(
                RECOVERY,
                Some(&alice),
                json!({ "current_password": PASSWORD }),
            )
            .await;
        let enrolled = h.enrol_through_console(&alice).await;
        let alice = enrolled.console;
        let b = db.backend;

        let wrong = h
            .post(
                RECOVERY,
                Some(&alice),
                json!({ "current_password": "nope" }),
            )
            .await;
        let fresh = h
            .post(
                RECOVERY,
                Some(&alice),
                json!({ "current_password": PASSWORD }),
            )
            .await;

        assert_eq!(early.status, 409, "{b}: nothing to regenerate");
        assert_eq!(wrong.status, 422, "{b}");
        assert_eq!(wrong.code(), "wrong_password", "{b}");
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
                json!({ "current_password": PASSWORD }),
            )
            .await;
        let after = client_login(&h, client_password_step("alice", PASSWORD)).await;
        assert_eq!(off.status, 200, "{b}: {:?}", off.body);
        assert_eq!(after.1["type"], "access_token", "{b}: the factor is gone");
    })
    .await;
}
