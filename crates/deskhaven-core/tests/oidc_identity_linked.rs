//! A linked identity at login (M4-09 rule 4): one linked by `email` is re-checked against
//! the email-link conditions at every login (a lowered ceiling, TOTP enabled, a promotion,
//! a changed and so unverified address, an IdP that stops asserting the address: refused,
//! on the client and the console path alike); an `explicit` or `provisioned` one is not
//! (a disabled account is still refused). `auth.tfa.enforce` constrains password logins
//! only (rule 1). Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, enable_totp};
use common::oidc_idp::TokenSpec;
use common::set;
use deskhaven_core::console_audit::TxGuard;
use deskhaven_core::oidc::{CallbackReport, Failure};
use deskhaven_core::users::{AccountEdit, ProfileUpdate, Role, Status, User};
use deskhaven_db::entity::user_identity;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use serde_json::json;

const EMAIL: &str = "alice@example.com";

fn verified(s: TokenSpec) -> TokenSpec {
    s.email(EMAIL, Some(true))
}

fn failure(report: &CallbackReport) -> Option<Failure> {
    report.outcome.as_ref().err().copied()
}

async fn login(env: &Env, subject: &str) -> CallbackReport {
    env.login(subject, verified).await.1
}

/// Client and console logins of `subject` are both refused as unlinked.
async fn assert_refused_on_both_paths(env: &Env, subject: &str, why: &str) {
    let client = login(env, subject).await;
    let console = env.console_login(subject, verified).await;
    assert_eq!(
        failure(&client),
        Some(Failure::NoLinkedAccount),
        "{why}: client"
    );
    assert_eq!(
        failure(&console),
        Some(Failure::NoLinkedAccount),
        "{why}: console"
    );
}

/// A local account with [`EMAIL`] linked to `subject` by email (a first login).
async fn email_linked(env: &Env, subject: &str) -> User {
    let alice = env.local_user("alice", Some(EMAIL)).await;
    assert_eq!(login(env, subject).await.outcome.unwrap().user.id, alice.id);
    alice
}

/// A local account linked to `subject` explicitly.
async fn explicitly_linked(env: &Env, subject: &str) -> User {
    let alice = env.local_user("alice", Some(EMAIL)).await;
    env.link(alice.id, subject, verified).await.outcome.unwrap();
    alice
}

#[tokio::test]
async fn an_email_link_is_refused_once_the_ceiling_is_lowered() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = email_linked(&env, "sub-a").await;
        set(&env.settings, "auth.oidc.link_email_max_role", json!("off")).await;

        assert_refused_on_both_paths(&env, "sub-a", &db.backend.to_string()).await;
        set(
            &env.settings,
            "auth.oidc.link_email_max_role",
            json!("user"),
        )
        .await;
        assert_eq!(
            login(&env, "sub-a").await.outcome.unwrap().user.id,
            alice.id
        );
    })
    .await;
}

#[tokio::test]
async fn an_email_link_is_refused_once_totp_is_enabled() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = email_linked(&env, "sub-a").await;
        set(
            &env.settings,
            "auth.oidc.link_email_max_role",
            json!("owner"),
        )
        .await;
        enable_totp(db, alice.id).await;

        assert_refused_on_both_paths(&env, "sub-a", &db.backend.to_string()).await;
    })
    .await;
}

#[tokio::test]
async fn an_email_link_is_refused_once_promoted_above_the_ceiling() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = email_linked(&env, "sub-a").await;
        env.users.set_role(alice.id, Role::Admin).await.unwrap();

        assert_refused_on_both_paths(&env, "sub-a", &db.backend.to_string()).await;
        set(
            &env.settings,
            "auth.oidc.link_email_max_role",
            json!("admin"),
        )
        .await;
        assert_eq!(
            login(&env, "sub-a").await.outcome.unwrap().user.id,
            alice.id
        );
    })
    .await;
}

#[tokio::test]
async fn an_email_link_is_refused_once_the_account_address_changes() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = email_linked(&env, "sub-a").await;
        let edit = AccountEdit {
            profile: ProfileUpdate {
                email: Some(Some("alice@new.example.com".to_owned())),
                ..ProfileUpdate::default()
            },
            role: None,
            status: None,
        };
        let changed = env
            .users
            .edit(alice.id, edit, env_guard(&env))
            .await
            .unwrap();

        assert!(!changed.email_verified, "the new address is unverified");
        assert_refused_on_both_paths(&env, "sub-a", &db.backend.to_string()).await;
    })
    .await;
}

fn env_guard(env: &Env) -> TxGuard<User> {
    TxGuard::new(&env.audit, |_, after: Option<&User>| {
        Ok(deskhaven_core::console_audit::NewConsoleAudit {
            actor: deskhaven_core::console_audit::Actor::Cli,
            action: "user.update".to_owned(),
            target_type: "user".to_owned(),
            target_id: after.map(|u| u.id.to_string()).unwrap_or_default(),
            before: None,
            after: None,
            ip: None,
        })
    })
}

#[tokio::test]
async fn an_email_link_is_refused_once_the_idp_stops_asserting_the_address() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        email_linked(&env, "sub-a").await;

        let unverified = env.login("sub-a", |s| s.email(EMAIL, Some(false))).await.1;
        let other = env
            .login("sub-a", |s| s.email("alice@elsewhere.example", Some(true)))
            .await
            .1;

        let b = db.backend;
        assert_eq!(failure(&unverified), Some(Failure::NoLinkedAccount), "{b}");
        assert_eq!(
            failure(&other),
            Some(Failure::NoLinkedAccount),
            "{b}: another address"
        );
    })
    .await;
}

#[tokio::test]
async fn an_explicit_link_survives_totp_promotion_and_a_changed_address() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = explicitly_linked(&env, "sub-a").await;
        enable_totp(db, alice.id).await;
        env.users.set_role(alice.id, Role::Owner).await.unwrap();
        set(&env.settings, "auth.oidc.link_email_max_role", json!("off")).await;

        let client = env.login("sub-a", |s| s).await.1;
        let console = env.console_login("sub-a", |s| s).await;

        let b = db.backend;
        assert_eq!(client.outcome.unwrap().user.id, alice.id, "{b}");
        assert_eq!(console.outcome.unwrap().user.id, alice.id, "{b}");
    })
    .await;
}

#[tokio::test]
async fn an_explicitly_linked_account_disabled_later_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = explicitly_linked(&env, "sub-a").await;
        env.users
            .set_status(alice.id, Status::Disabled)
            .await
            .unwrap();

        let report = login(&env, "sub-a").await;

        assert_eq!(failure(&report), Some(Failure::Disabled), "{}", db.backend);
    })
    .await;
}

/// Rule 1: enforcement covers password logins only. A linked local account without TOTP
/// signs in through the IdP while `auth.tfa.enforce = all`.
#[tokio::test]
async fn tfa_enforcement_does_not_refuse_oidc_logins() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = email_linked(&env, "sub-a").await;
        let bob = env.local_user("bob", Some("bob@example.com")).await;
        set(&env.settings, "auth.tfa.enforce", json!("all")).await;

        let client = login(&env, "sub-a").await;
        let console = env.console_login("sub-a", verified).await;
        let new_link = env
            .login("sub-b", |s| s.email("bob@example.com", Some(true)))
            .await
            .1;

        let b = db.backend;
        assert_eq!(client.outcome.unwrap().user.id, alice.id, "{b}");
        assert_eq!(console.outcome.unwrap().user.id, alice.id, "{b}");
        assert_eq!(new_link.outcome.unwrap().user.id, bob.id, "{b}");
    })
    .await;
}

/// An identity linked to an LDAP account before LDAP accounts were excluded (`email`, the
/// migration's default): the directory decides on that account, an OIDC login alone does not.
#[tokio::test]
async fn an_existing_email_link_to_an_ldap_account_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        set(
            &env.settings,
            "auth.oidc.link_email_max_role",
            json!("owner"),
        )
        .await;
        let lara = env.ldap_user("lara", EMAIL).await;
        user_identity::ActiveModel {
            user_id: Set(lara.id),
            provider_id: Set(env.provider.id),
            subject: Set("sub-l".to_owned()),
            email_verified: Set(true),
            email: Set(Some(EMAIL.to_owned())),
            created_at: Set(deskhaven_db::time::now()),
            linked_via: Set(user_identity::LinkedVia::Email),
            ..Default::default()
        }
        .insert(&db.conn)
        .await
        .unwrap();

        let client = login(&env, "sub-l").await;

        assert_eq!(
            failure(&client),
            Some(Failure::NoLinkedAccount),
            "{}",
            db.backend
        );
        assert_eq!(
            user_identity::Entity::find()
                .all(&db.conn)
                .await
                .unwrap()
                .len(),
            1
        );
    })
    .await;
}

/// Accounts the IdP provisioned are the IdP's: a role granted later is not a bypass.
#[tokio::test]
async fn a_provisioned_oidc_account_promoted_later_still_signs_in() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let first = env.login("sub-o", |s| s).await.1;
        let id = first.outcome.unwrap().user.id;
        env.users.set_role(id, Role::Admin).await.unwrap();

        let again = env.login("sub-o", |s| s).await.1;

        assert_eq!(again.outcome.unwrap().user.id, id, "{}", db.backend);
    })
    .await;
}
