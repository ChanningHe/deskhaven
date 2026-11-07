//! Listing and removing a user's OIDC identities (M4-09): the owner of the account or an
//! administrator (the caller's guard decides) unlinks; the last identity of an `oidc`
//! account stays (it has no other way in); an identity of another account is not found
//! through the self-service path; a refused guard changes nothing. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, PROVIDER};
use common::oidc_idp::TokenSpec;
use deskhaven_core::console_audit::{Refused, TxGuard};
use deskhaven_core::oidc::{Failure, IdentityError, NewProvider};
use deskhaven_db::entity::user_identity::LinkedVia;
use deskhaven_db::entity::{console_audit, user_identity};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{ActiveModelTrait, EntityTrait, PaginatorTrait};

async fn identity_count(db: &TestDb) -> u64 {
    user_identity::Entity::find().count(&db.conn).await.unwrap()
}

#[tokio::test]
async fn a_user_lists_and_unlinks_an_identity_and_the_login_stops_working() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        env.link(alice.id, "sub-a", |s| s).await.outcome.unwrap();

        let listed = env.oidc.identities_of(alice.id).await.unwrap();
        let guard = env.link_guard(alice.id, "user.identity_unlink");
        env.oidc
            .unlink_identity(listed[0].id, Some(alice.id), guard)
            .await
            .unwrap();
        let login = env.login("sub-a", |s| s).await.1;

        let b = db.backend;
        assert_eq!(listed.len(), 1, "{b}");
        assert_eq!(listed[0].provider_name, PROVIDER, "{b}");
        assert_eq!(
            listed[0].provider_display_name, env.provider.display_name,
            "{b}"
        );
        assert_eq!(listed[0].linked_via, LinkedVia::Explicit, "{b}");
        assert_eq!(identity_count(db).await, 0, "{b}");
        assert_eq!(
            login.outcome.err(),
            Some(Failure::NoLinkedAccount),
            "{b}: unlinked"
        );
        let actions: Vec<String> = console_audit::Entity::find()
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.action)
            .collect();
        assert_eq!(
            actions,
            ["user.identity_link", "user.identity_unlink"],
            "{b}"
        );
    })
    .await;
}

/// Unlinking any identity (email, explicit or provisioned) marks the account's verified
/// address unverified in the same transaction: otherwise the same IdP account would link
/// again by email at its next login (an admin recovering a compromised account unlinks it
/// and resets the second factor, which removes the last email-link blocker).
#[tokio::test]
async fn unlinking_any_identity_unverifies_the_address_so_nothing_relinks_by_email() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let verified = |email: &'static str| move |s: TokenSpec| s.email(email, Some(true));
        let alice = env.local_user("alice", Some("alice@example.com")).await;
        let bob = env.local_user("bob", Some("bob@example.com")).await;
        env.login("sub-a", verified("alice@example.com"))
            .await
            .1
            .outcome
            .unwrap();
        env.link(bob.id, "sub-b", verified("bob@example.com"))
            .await
            .outcome
            .unwrap();
        let olga = env.login("sub-o", verified("olga@example.com")).await.1;
        let olga = olga.outcome.unwrap().user;
        second_identity(&env, db, olga.id).await;
        let guard = || env.link_guard(1, "user.identity_unlink");

        for user_id in [alice.id, bob.id, olga.id] {
            let id = env.oidc.identities_of(user_id).await.unwrap()[0].id;
            env.oidc.unlink_identity(id, None, guard()).await.unwrap();
        }
        let alice_again = env.login("sub-a", verified("alice@example.com")).await.1;
        let bob_again = env.login("sub-b", verified("bob@example.com")).await.1;

        let b = db.backend;
        for user_id in [alice.id, bob.id, olga.id] {
            let user = env.users.get(user_id).await.unwrap();
            assert!(!user.email_verified, "{b}: {}", user.username);
        }
        assert_eq!(
            alice_again.outcome.err(),
            Some(Failure::NoLinkedAccount),
            "{b}"
        );
        assert_eq!(
            bob_again.outcome.err(),
            Some(Failure::NoLinkedAccount),
            "{b}: explicit"
        );
        assert_eq!(identity_count(db).await, 1, "{b}: olga's second identity");
    })
    .await;
}

/// A second (explicit) identity of `user_id` at another provider.
async fn second_identity(env: &Env, db: &TestDb, user_id: i64) {
    let mut other = NewProvider::new("other", env.idp.issuer.clone(), "other-client");
    other.client_secret = Some("s".to_owned());
    let other = env.oidc.create_provider(other).await.unwrap();
    user_identity::ActiveModel {
        user_id: sea_orm::ActiveValue::Set(user_id),
        provider_id: sea_orm::ActiveValue::Set(other.id),
        subject: sea_orm::ActiveValue::Set(format!("other-{user_id}")),
        email_verified: sea_orm::ActiveValue::Set(false),
        email: sea_orm::ActiveValue::Set(None),
        created_at: sea_orm::ActiveValue::Set(deskhaven_db::time::now()),
        linked_via: sea_orm::ActiveValue::Set(LinkedVia::Explicit),
        ..Default::default()
    }
    .insert(&db.conn)
    .await
    .unwrap();
}

#[tokio::test]
async fn the_last_identity_of_an_oidc_account_cannot_be_unlinked() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let olga = env.login("sub-o", |s| s).await.1.outcome.unwrap().user;
        let mut second = NewProvider::new("other", env.idp.issuer.clone(), "other-client");
        second.client_secret = Some("s".to_owned());
        let other = env.oidc.create_provider(second).await.unwrap();

        let only = env.oidc.identities_of(olga.id).await.unwrap();
        let refused = env
            .oidc
            .unlink_identity(only[0].id, None, env.link_guard(1, "user.identity_unlink"))
            .await;
        user_identity::ActiveModel {
            user_id: sea_orm::ActiveValue::Set(olga.id),
            provider_id: sea_orm::ActiveValue::Set(other.id),
            subject: sea_orm::ActiveValue::Set("sub-o2".to_owned()),
            email_verified: sea_orm::ActiveValue::Set(false),
            email: sea_orm::ActiveValue::Set(None),
            created_at: sea_orm::ActiveValue::Set(deskhaven_db::time::now()),
            linked_via: sea_orm::ActiveValue::Set(LinkedVia::Explicit),
            ..Default::default()
        }
        .insert(&db.conn)
        .await
        .unwrap();
        let with_two = env
            .oidc
            .unlink_identity(only[0].id, None, env.link_guard(1, "user.identity_unlink"))
            .await;

        let b = db.backend;
        assert!(
            matches!(refused, Err(IdentityError::LastIdentity)),
            "{b}: {refused:?}"
        );
        assert!(with_two.is_ok(), "{b}: {with_two:?}");
        assert_eq!(identity_count(db).await, 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn another_accounts_identity_is_not_found_through_self_service() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let bob = env.local_user("bob", None).await;
        env.link(alice.id, "sub-a", |s| s).await.outcome.unwrap();
        let id = env.oidc.identities_of(alice.id).await.unwrap()[0].id;

        let as_bob = env
            .oidc
            .unlink_identity(
                id,
                Some(bob.id),
                env.link_guard(bob.id, "user.identity_unlink"),
            )
            .await;
        let unknown = env
            .oidc
            .unlink_identity(
                id + 1000,
                None,
                env.link_guard(bob.id, "user.identity_unlink"),
            )
            .await;

        let b = db.backend;
        assert!(matches!(as_bob, Err(IdentityError::NotFound)), "{b}");
        assert!(matches!(unknown, Err(IdentityError::NotFound)), "{b}");
        assert_eq!(identity_count(db).await, 1, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_refusing_guard_changes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        env.link(alice.id, "sub-a", |s| s).await.outcome.unwrap();
        let id = env.oidc.identities_of(alice.id).await.unwrap()[0].id;
        let refuse = TxGuard::new(&env.audit, |_, _| Err(Refused::new("owner_required")));

        let refused = env.oidc.unlink_identity(id, None, refuse).await;

        let b = db.backend;
        assert!(
            matches!(refused, Err(IdentityError::Refused(r)) if r.code == "owner_required"),
            "{b}"
        );
        assert_eq!(identity_count(db).await, 1, "{b}");
    })
    .await;
}
