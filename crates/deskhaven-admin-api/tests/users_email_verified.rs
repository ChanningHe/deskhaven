//! Administrators mark an account's email verified (M4-09): `PATCH /users/{id}` with
//! `email_verified`, under the account rules (only an owner touches an owner), audited as
//! `user.update`; a new address still resets the flag; an account without an address cannot
//! be marked. Both backends.
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

#[tokio::test]
async fn an_admin_marks_an_address_verified_and_a_new_address_resets_it() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adam", Role::Admin).await;
        let bob = with_email(&h, "bob", Role::User).await;
        let mark = json!({ "email_verified": true });
        let b = db.backend;

        let marked = h.write(Method::PATCH, &path(bob), &admin, mark).await;
        let moved = h
            .write(
                Method::PATCH,
                &path(bob),
                &admin,
                json!({ "email": "bob@new.example.com" }),
            )
            .await;
        let both = h
            .write(
                Method::PATCH,
                &path(bob),
                &admin,
                json!({ "email": "bob@third.example.com", "email_verified": true }),
            )
            .await;

        assert_eq!(marked.status, 200, "{b}: {}", marked.body);
        assert_eq!(marked.body["email_verified"], true, "{b}");
        assert_eq!(
            moved.body["email_verified"], false,
            "{b}: a new address is unverified"
        );
        assert_eq!(
            both.body["email_verified"], true,
            "{b}: set explicitly with it"
        );
        let rows = h.audit_rows().await;
        assert_eq!(rows[0].action, "user.update", "{b}");
        let after: Value = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
        let before: Value = serde_json::from_str(rows[0].before.as_deref().unwrap()).unwrap();
        assert_eq!(
            (
                before["email_verified"].clone(),
                after["email_verified"].clone()
            ),
            (json!(false), json!(true)),
            "{b}"
        );
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
        let mark = || json!({ "email_verified": true });
        let b = db.backend;

        let by_admin = h.write(Method::PATCH, &path(olive), &admin, mark()).await;
        let by_user = h.write(Method::PATCH, &path(bob), &user, mark()).await;
        let by_owner = h.write(Method::PATCH, &path(olive), &owner, mark()).await;

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
            .write(
                Method::PATCH,
                &path(bob),
                &admin,
                json!({ "email_verified": true }),
            )
            .await;

        let b = db.backend;
        assert_eq!(reply.status, 422, "{b}: {}", reply.body);
        assert!(
            reply.body["error"]["details"]["fields"]["email_verified"].is_array(),
            "{b}: {}",
            reply.body
        );
    })
    .await;
}
