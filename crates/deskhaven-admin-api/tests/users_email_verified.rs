//! Administrators mark an account's email verified (M4-09): `PATCH /users/{id}` with
//! `email_verified: true` and the `expected_email` they checked (a different stored address
//! is 409 `conflict`, reason `email_changed`), never together with a new `email` (a separate
//! step); under the account rules (only an owner touches an owner), audited as
//! `user.update`; a new address resets the flag; an account without an address cannot be
//! marked. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::harness;
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::{Credential, NewUser, Password, Role};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

async fn with_email(h: &common::Harness, username: &str, role: Role) -> i64 {
    let new = NewUser {
        email: Some(format!("{username}@example.com")),
        role,
        ..NewUser::new(
            username,
            Credential::Local {
                password: Password::new(common::PASSWORD),
            },
        )
    };
    h.state.users.create(new).await.unwrap().id
}

fn path(id: i64) -> String {
    format!("/admin/api/v1/users/{id}")
}

fn mark(expected: &str) -> Value {
    json!({ "email_verified": true, "expected_email": expected })
}

fn invalid_field(reply: &common::Reply, field: &str) -> bool {
    reply.status == 422 && reply.body["error"]["details"]["fields"][field].is_array()
}

#[tokio::test]
async fn an_admin_marks_the_checked_address_verified_and_a_new_address_resets_it() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adam", Role::Admin).await;
        let bob = with_email(&h, "bob", Role::User).await;
        let b = db.backend;

        let marked = h
            .write(Method::PATCH, &path(bob), &admin, mark("BOB@example.com"))
            .await;
        let moved = h
            .write(
                Method::PATCH,
                &path(bob),
                &admin,
                json!({ "email": "bob@new.example.com" }),
            )
            .await;

        assert_eq!(marked.status, 200, "{b}: {}", marked.body);
        assert_eq!(marked.body["email_verified"], true, "{b}");
        assert_eq!(moved.body["email_verified"], false, "{b}: a new address");
        let rows = h.audit_rows().await;
        assert_eq!(rows[0].action, "user.update", "{b}");
        let after: Value = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
        let before: Value = serde_json::from_str(rows[0].before.as_deref().unwrap()).unwrap();
        assert_eq!(before["email_verified"], json!(false), "{b}");
        assert_eq!(after["email_verified"], json!(true), "{b}");
    })
    .await;
}

#[tokio::test]
async fn marking_needs_the_checked_address_and_never_comes_with_a_new_one() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adam", Role::Admin).await;
        let bob = with_email(&h, "bob", Role::User).await;
        let b = db.backend;

        let with_new = h
            .write(
                Method::PATCH,
                &path(bob),
                &admin,
                json!({ "email": "bob@third.example.com", "email_verified": true, "expected_email": "bob@third.example.com" }),
            )
            .await;
        let without_expected = h
            .write(Method::PATCH, &path(bob), &admin, json!({ "email_verified": true }))
            .await;
        let changed = h
            .write(Method::PATCH, &path(bob), &admin, mark("bob@old.example.com"))
            .await;

        assert!(invalid_field(&with_new, "email_verified"), "{b}: {}", with_new.body);
        assert!(
            invalid_field(&without_expected, "expected_email"),
            "{b}: {}",
            without_expected.body
        );
        assert_eq!(changed.status, 409, "{b}: {}", changed.body);
        assert_eq!(changed.body["error"]["code"], "conflict", "{b}");
        assert_eq!(changed.body["error"]["details"]["reason"], "email_changed", "{b}");
        assert!(!h.state.users.get(bob).await.unwrap().email_verified, "{b}");
    })
    .await;
}

#[tokio::test]
async fn only_an_owner_marks_an_owner_and_users_may_not_mark_at_all() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adam", Role::Admin).await;
        let owner = h.console("root", Role::Owner).await;
        let user = h.console("ursula", Role::User).await;
        let olive = with_email(&h, "olive", Role::Owner).await;
        let bob = with_email(&h, "bob", Role::User).await;
        let b = db.backend;

        let by_admin = h
            .write(
                Method::PATCH,
                &path(olive),
                &admin,
                mark("olive@example.com"),
            )
            .await;
        let by_user = h
            .write(Method::PATCH, &path(bob), &user, mark("bob@example.com"))
            .await;
        let by_owner = h
            .write(
                Method::PATCH,
                &path(olive),
                &owner,
                mark("olive@example.com"),
            )
            .await;

        assert_eq!(by_admin.status, 403, "{b}");
        assert_eq!(by_admin.body["error"]["code"], "owner_required", "{b}");
        assert_eq!(by_user.status, 403, "{b}");
        assert_eq!(by_owner.status, 200, "{b}: {}", by_owner.body);
        assert_eq!(by_owner.body["email_verified"], true, "{b}");
    })
    .await;
}

#[tokio::test]
async fn an_account_without_an_address_cannot_be_marked() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adam", Role::Admin).await;
        let bob = h.create_user("bob", Role::User).await;

        let reply = h
            .write(Method::PATCH, &path(bob), &admin, mark("bob@example.com"))
            .await;

        let b = db.backend;
        assert!(
            invalid_field(&reply, "email_verified"),
            "{b}: {}",
            reply.body
        );
    })
    .await;
}
