//! Which account an OIDC login reaches (06 §3.4): the `(provider, sub)` identity, email
//! linking by `email_verified` true / false / missing and `auth.oidc.link_email_max_role`,
//! auto-provisioning (username rules, collisions, email, group mapping, admin group) and
//! inactive accounts. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::Env;
use common::oidc_idp::TokenSpec;
use common::set;
use deskhaven_core::groups::{GroupService, NewGroup};
use deskhaven_core::oidc::{CallbackReport, Failure};
use deskhaven_core::users::{AuthSource, ProfileUpdate, Role, Status};
use deskhaven_db::entity::{group_member, user, user_identity};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::json;

fn user_id(report: &CallbackReport) -> i64 {
    match &report.outcome {
        Ok(login) => login.user.id,
        Err(failure) => panic!("login failed: {failure:?}"),
    }
}

fn failure(report: &CallbackReport) -> Option<Failure> {
    report.outcome.as_ref().err().copied()
}

async fn identities(db: &TestDb) -> Vec<user_identity::Model> {
    user_identity::Entity::find().all(&db.conn).await.unwrap()
}

#[tokio::test]
async fn verified_email_links_the_existing_account_once_then_the_identity_is_used() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", Some("Alice@Example.com")).await;

        let (_, first) = env
            .login("sub-a", |s| s.email("alice@example.com", Some(true)))
            .await;
        // Later logins go through the identity (no second link), and its address follows
        // the token's verified one.
        let (_, second) = env
            .login("sub-a", |s| s.email("ALICE@example.com", Some(true)))
            .await;

        assert_eq!(user_id(&first), alice.id, "{}", db.backend);
        assert_eq!(user_id(&second), alice.id);
        assert_eq!(first.subject.as_deref(), Some("sub-a"));
        let linked = identities(db).await;
        assert_eq!(linked.len(), 1);
        assert_eq!(
            (linked[0].user_id, linked[0].provider_id),
            (alice.id, env.provider.id)
        );
        assert_eq!(linked[0].subject, "sub-a");
        assert_eq!(linked[0].email.as_deref(), Some("ALICE@example.com"));
        assert!(linked[0].email_verified);
        // Still the local account it was.
        let row = env.users.get(alice.id).await.unwrap();
        assert_eq!(row.auth_source, AuthSource::Local);
    })
    .await;
}

#[tokio::test]
async fn unverified_or_missing_email_verification_never_links_nor_provisions_a_duplicate() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let alice = env.local_user("alice", Some("alice@example.com")).await;

        let (_, unverified) = env
            .login("sub-a", |s| s.email("alice@example.com", Some(false)))
            .await;
        let (_, missing) = env
            .login("sub-b", |s| s.email("ALICE@example.com", None))
            .await;

        assert_eq!(
            failure(&unverified),
            Some(Failure::NoLinkedAccount),
            "{}",
            db.backend
        );
        assert_eq!(failure(&missing), Some(Failure::NoLinkedAccount));
        assert!(identities(db).await.is_empty());
        let users = deskhaven_db::entity::user::Entity::find()
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!(
            users, 1,
            "no account was provisioned next to {}",
            alice.username
        );
    })
    .await;
}

#[tokio::test]
async fn verified_email_does_not_link_when_linking_is_turned_off() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.local_user("alice", Some("alice@example.com")).await;
        set(&env.settings, "auth.oidc.link_email_max_role", json!("off")).await;

        let (_, report) = env
            .login("sub-a", |s| s.email("alice@example.com", Some(true)))
            .await;

        assert_eq!(
            failure(&report),
            Some(Failure::NoLinkedAccount),
            "{}",
            db.backend
        );
        assert!(identities(db).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn without_provisioning_an_unknown_identity_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;

        let (_, with_email) = env
            .login("sub-x", |s| s.email("nobody@example.com", Some(true)))
            .await;
        let (_, without) = env.login("sub-y", |s| s).await;

        assert_eq!(
            failure(&with_email),
            Some(Failure::NoLinkedAccount),
            "{}",
            db.backend
        );
        assert_eq!(failure(&without), Some(Failure::NoLinkedAccount));
    })
    .await;
}

#[tokio::test]
async fn provisioning_creates_an_active_oidc_account_with_the_asserted_profile() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;

        let (_, report) = env
            .login("sub-new", |s| TokenSpec {
                preferred_username: Some("bob.smith".to_owned()),
                name: Some("Bob Smith".to_owned()),
                ..s.email("Bob@Example.com", Some(true))
            })
            .await;
        let (_, again) = env.login("sub-new", |s| s).await;

        let bob = env.users.get(user_id(&report)).await.unwrap();
        assert_eq!(bob.username, "bob.smith", "{}", db.backend);
        assert_eq!(bob.display_name, "Bob Smith");
        assert_eq!(bob.email.as_deref(), Some("Bob@Example.com"));
        assert!(bob.email_verified);
        assert_eq!(
            (bob.auth_source, bob.role, bob.status),
            (AuthSource::Oidc, Role::User, Status::Active)
        );
        assert_eq!(user_id(&again), bob.id);
        assert_eq!(identities(db).await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn provisioned_usernames_follow_the_username_rules_and_avoid_existing_accounts() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        env.local_user("alice", None).await;
        env.local_user("Alice-2", None).await;

        let (_, clash) = env
            .login("s1", |s| TokenSpec {
                preferred_username: Some("ALICE".to_owned()),
                ..s
            })
            .await;
        let (_, unicode) = env
            .login("s2", |s| TokenSpec {
                preferred_username: Some("Zoë Smith/ops".to_owned()),
                ..s
            })
            .await;
        let (_, from_email) = env
            .login("s3", |s| s.email("carol+x@example.com", Some(false)))
            .await;
        let (_, from_sub) = env
            .login("Abc_123-xyz", |s| TokenSpec {
                preferred_username: Some("张三".to_owned()),
                ..s
            })
            .await;
        let long = "l".repeat(100);
        let (_, long_name) = env
            .login("s5", |s| TokenSpec {
                preferred_username: Some(long.clone()),
                ..s
            })
            .await;

        let name = |r: &CallbackReport| r.outcome.as_ref().unwrap().user.username.clone();
        // Never the existing "alice" (case-insensitively), never "Alice-2".
        assert_eq!(name(&clash), "ALICE-3", "{}", db.backend);
        assert_eq!(name(&unicode), "ZoSmithops");
        // `+` is outside the charset; an unverified email is not stored.
        assert_eq!(name(&from_email), "carolx@example.com");
        assert_eq!(from_email.outcome.as_ref().unwrap().user.email, None);
        assert_eq!(name(&from_sub), "Abc_123-xyz");
        assert_eq!(name(&long_name), "l".repeat(64));
    })
    .await;
}

#[tokio::test]
async fn group_claims_map_to_existing_groups_and_the_admin_group_grants_admin() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| {
            p.auto_provision = true;
            p.group_claim = Some("groups".to_owned());
            p.admin_group = Some("rd-admins".to_owned());
        })
        .await;
        let groups = GroupService::new(db.conn.clone());
        let ops = groups.create(NewGroup::new("ops")).await.unwrap();
        groups.create(NewGroup::new("unused")).await.unwrap();
        let with_groups = |groups: serde_json::Value| {
            move |mut s: TokenSpec| {
                s.extra.insert("groups".to_owned(), groups);
                s
            }
        };

        let (_, admin) = env
            .login(
                "s-admin",
                with_groups(json!(["ops", "rd-admins", "not-a-group"])),
            )
            .await;
        let (_, plain) = env.login("s-plain", with_groups(json!("ops"))).await;
        let (_, odd) = env.login("s-odd", with_groups(json!({"ops": 1}))).await;

        let admin = env.users.get(user_id(&admin)).await.unwrap();
        let plain = env.users.get(user_id(&plain)).await.unwrap();
        let odd = env.users.get(user_id(&odd)).await.unwrap();
        assert_eq!(admin.role, Role::Admin, "{}", db.backend);
        assert_eq!(plain.role, Role::User);
        assert_eq!(odd.role, Role::User);
        let members: Vec<i64> = group_member::Entity::find()
            .filter(group_member::Column::GroupId.eq(ops.id))
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|m| m.user_id)
            .collect();
        assert_eq!(members.len(), 2);
        assert!(members.contains(&admin.id) && members.contains(&plain.id));
        assert_eq!(
            group_member::Entity::find().count(&db.conn).await.unwrap(),
            2
        );
    })
    .await;
}

#[tokio::test]
async fn disabled_and_unverified_accounts_are_refused_and_not_linked() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let dave = env.local_user("dave", Some("dave@example.com")).await;
        env.users
            .set_status(dave.id, Status::Disabled)
            .await
            .unwrap();
        let erin = env.local_user("erin", Some("erin@example.com")).await;
        env.users
            .set_status(erin.id, Status::Unverified)
            .await
            .unwrap();

        let (_, disabled) = env
            .login("s-dave", |s| s.email("dave@example.com", Some(true)))
            .await;
        let (_, unverified) = env
            .login("s-erin", |s| s.email("erin@example.com", Some(true)))
            .await;

        assert_eq!(
            failure(&disabled),
            Some(Failure::Disabled),
            "{}",
            db.backend
        );
        assert_eq!(failure(&unverified), Some(Failure::Unverified));
        assert!(identities(db).await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_linked_account_disabled_later_is_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (_, first) = env.login("s-1", |s| s).await;
        let id = user_id(&first);
        env.users.set_status(id, Status::Disabled).await.unwrap();

        let (_, second) = env.login("s-1", |s| s).await;

        assert_eq!(failure(&second), Some(Failure::Disabled), "{}", db.backend);
    })
    .await;
}

/// H1: an IdP that lets users set any (unverified) address cannot squat it.
#[tokio::test]
async fn an_unverified_email_is_never_stored_so_it_cannot_be_squatted() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;

        // A: the attacker claims the CEO's address, unverified.
        let (_, squatter) = env
            .login("attacker", |s| TokenSpec {
                preferred_username: Some("mallory".to_owned()),
                ..s.email("ceo@corp.example", Some(false))
            })
            .await;
        // B: the CEO logs in later with the verified address.
        let (_, ceo) = env
            .login("ceo-sub", |s| TokenSpec {
                preferred_username: Some("ceo".to_owned()),
                ..s.email("ceo@corp.example", Some(true))
            })
            .await;

        let mallory = env.users.get(user_id(&squatter)).await.unwrap();
        let ceo = env.users.get(user_id(&ceo)).await.unwrap();
        assert_ne!(mallory.id, ceo.id, "{}", db.backend);
        assert_eq!(mallory.email, None);
        assert!(!mallory.email_verified);
        assert_eq!(ceo.email.as_deref(), Some("ceo@corp.example"));
        assert!(ceo.email_verified);
        let rows = identities(db).await;
        let attacker = rows.iter().find(|i| i.user_id == mallory.id).unwrap();
        assert_eq!(attacker.email, None);
        assert!(!attacker.email_verified);
    })
    .await;
}

/// H1: an account whose own address is unverified is never an auto-link target.
#[tokio::test]
async fn verified_email_does_not_link_into_an_account_with_an_unverified_address() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let squatter = env.local_user("squatter", Some("ceo@corp.example")).await;
        let unverify = ProfileUpdate {
            email_verified: Some(false),
            ..ProfileUpdate::default()
        };
        env.users
            .update_profile(squatter.id, unverify)
            .await
            .unwrap();

        let (_, report) = env
            .login("ceo-sub", |s| s.email("ceo@corp.example", Some(true)))
            .await;

        assert_eq!(
            failure(&report),
            Some(Failure::NoLinkedAccount),
            "{}",
            db.backend
        );
        assert!(identities(db).await.is_empty());
    })
    .await;
}

/// M2: an OIDC login must not bypass a local second factor, nor reach a privileged account
/// by email alone.
#[tokio::test]
async fn verified_email_never_links_into_two_factor_admin_or_owner_accounts() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let tina = env.local_user("tina", Some("tina@example.com")).await;
        let mut row: user::ActiveModel = user::Entity::find_by_id(tina.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap()
            .into();
        // Enrolled (the seed value is irrelevant here: only "has TOTP" matters).
        row.totp_secret_enc = Set(Some("v1:placeholder".to_owned()));
        row.totp_enabled_at = Set(Some(deskhaven_db::time::now()));
        row.update(&db.conn).await.unwrap();
        let adam = env.local_user("adam", Some("adam@example.com")).await;
        env.users.set_role(adam.id, Role::Admin).await.unwrap();
        let olive = env.local_user("olive", Some("olive@example.com")).await;
        env.users.set_role(olive.id, Role::Owner).await.unwrap();

        for (sub, email) in [
            ("s-tina", "tina@example.com"),
            ("s-adam", "adam@example.com"),
            ("s-olive", "olive@example.com"),
        ] {
            let (_, report) = env.login(sub, |s| s.email(email, Some(true))).await;
            assert_eq!(
                failure(&report),
                Some(Failure::NoLinkedAccount),
                "{}: {email}",
                db.backend
            );
        }
        assert!(identities(db).await.is_empty());
    })
    .await;
}
