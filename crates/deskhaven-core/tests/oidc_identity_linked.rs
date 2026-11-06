//! An identity linked to a local or LDAP account (06 §3.4) signs in only while an OIDC
//! login bypasses nothing on it: TOTP enabled or a role above `user` after the link, or an
//! LDAP account at all, is refused on the client and the console path alike. LDAP accounts
//! are never linked by email. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, PROVIDER};
use common::oidc_idp::{AUTH_CODE, AuthRequest};
use deskhaven_core::oidc::{CallbackParams, CallbackReport, Failure};
use deskhaven_core::users::{Credential, NewUser, Role, User};
use deskhaven_db::entity::{user, user_identity};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};

const EMAIL: &str = "alice@example.com";

fn failure(report: &CallbackReport) -> Option<Failure> {
    report.outcome.as_ref().err().copied()
}

async fn login(env: &Env, subject: &str) -> CallbackReport {
    env.login(subject, |s| s.email(EMAIL, Some(true))).await.1
}

/// The console flow of `subject`, completed in the browser that started it.
async fn console_login(env: &Env, subject: &str) -> CallbackReport {
    let start = env.oidc.begin_console(PROVIDER).await.unwrap();
    let request = AuthRequest::parse(&start.url);
    let spec = env.spec(subject, &request).email(EMAIL, Some(true));
    env.idp.mount_token(&spec).await;
    let params = CallbackParams {
        state: Some(&request.state),
        code: Some(AUTH_CODE),
        error: None,
    };
    env.oidc
        .complete_console(&params, Some(start.binding.expose()))
        .await
        .unwrap()
}

/// A local account with [`EMAIL`] linked to `subject` by a first (successful) login.
async fn linked_local(env: &Env, subject: &str) -> User {
    let alice = env.local_user("alice", Some(EMAIL)).await;
    let first = login(env, subject).await;
    assert_eq!(first.outcome.unwrap().user.id, alice.id);
    alice
}

/// Enrols TOTP on the row directly (only "has TOTP" matters here).
async fn enable_totp(db: &TestDb, user_id: i64) {
    let mut row: user::ActiveModel = user::Entity::find_by_id(user_id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
        .into();
    row.totp_secret_enc = Set(Some("v1:placeholder".to_owned()));
    row.totp_enabled_at = Set(Some(deskhaven_db::time::now()));
    row.update(&db.conn).await.unwrap();
}

async fn ldap_user(env: &Env) -> User {
    let new = NewUser {
        email: Some(EMAIL.to_owned()),
        email_verified: true,
        ..NewUser::new("lara", Credential::Ldap)
    };
    env.users.create(new).await.unwrap()
}

#[tokio::test]
async fn a_linked_local_account_that_enables_totp_later_is_refused_on_both_paths() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = linked_local(&env, "sub-a").await;
        enable_totp(db, alice.id).await;

        let client = login(&env, "sub-a").await;
        let console = console_login(&env, "sub-a").await;

        let b = db.backend;
        assert_eq!(failure(&client), Some(Failure::NoLinkedAccount), "{b}");
        assert_eq!(failure(&console), Some(Failure::NoLinkedAccount), "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_linked_local_account_promoted_later_is_refused_on_both_paths() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = linked_local(&env, "sub-a").await;
        env.users.set_role(alice.id, Role::Admin).await.unwrap();

        let client = login(&env, "sub-a").await;
        let console = console_login(&env, "sub-a").await;
        env.users.set_role(alice.id, Role::User).await.unwrap();
        let demoted = login(&env, "sub-a").await;

        let b = db.backend;
        assert_eq!(failure(&client), Some(Failure::NoLinkedAccount), "{b}");
        assert_eq!(failure(&console), Some(Failure::NoLinkedAccount), "{b}");
        assert_eq!(demoted.outcome.unwrap().user.id, alice.id, "{b}: demoted");
    })
    .await;
}

#[tokio::test]
async fn an_ldap_account_is_never_linked_by_email() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        ldap_user(&env).await;

        let client = login(&env, "sub-l").await;
        let console = console_login(&env, "sub-m").await;

        let b = db.backend;
        assert_eq!(failure(&client), Some(Failure::NoLinkedAccount), "{b}");
        assert_eq!(failure(&console), Some(Failure::NoLinkedAccount), "{b}");
        let rows = user_identity::Entity::find().all(&db.conn).await.unwrap();
        assert!(rows.is_empty(), "{b}");
    })
    .await;
}

/// An identity linked to an LDAP account before LDAP accounts were excluded: the
/// directory decides on that account, an OIDC login alone does not.
#[tokio::test]
async fn an_existing_link_to_an_ldap_account_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let lara = ldap_user(&env).await;
        user_identity::ActiveModel {
            user_id: Set(lara.id),
            provider_id: Set(env.provider.id),
            subject: Set("sub-l".to_owned()),
            email_verified: Set(true),
            email: Set(Some(EMAIL.to_owned())),
            created_at: Set(deskhaven_db::time::now()),
            ..Default::default()
        }
        .insert(&db.conn)
        .await
        .unwrap();

        let client = login(&env, "sub-l").await;

        let b = db.backend;
        assert_eq!(failure(&client), Some(Failure::NoLinkedAccount), "{b}");
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
