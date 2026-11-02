//! Group endpoints: list, detail, create, update, delete, members (list / add / remove),
//! config-managed groups read-only, audit rows, role gate. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::entity::group;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use serde_json::{Value, json};

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

async fn create(h: &Harness, console: &Console, name: &str) -> i64 {
    let body = json!({ "name": name, "note": "n" });
    let reply = h
        .write(Method::POST, "/admin/api/v1/groups", console, body)
        .await;
    assert_eq!(reply.status, 201, "{}", reply.body);
    reply.body["id"].as_i64().unwrap()
}

fn names(reply: &Value) -> Vec<&str> {
    let items = reply["items"].as_array().unwrap();
    items.iter().map(|g| g["name"].as_str().unwrap()).collect()
}

fn snapshot(text: Option<&str>) -> Value {
    serde_json::from_str(text.unwrap()).unwrap()
}

#[tokio::test]
async fn groups_are_created_listed_updated_and_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let ops = create(&h, &admin, "ops").await;
        create(&h, &admin, "dev").await;
        create(&h, &admin, "ops-night").await;

        let listed = h
            .get("/admin/api/v1/groups?sort=-name&page_size=10", &admin)
            .await;
        assert_eq!(names(&listed.body), ["ops-night", "ops", "dev"]);
        let found = h.get("/admin/api/v1/groups?q=OPS", &admin).await;
        assert_eq!(found.body["total"], 2);
        let bad = h
            .get("/admin/api/v1/groups?sort=member_count", &admin)
            .await;
        assert_eq!(bad.status, 422);

        let path = format!("/admin/api/v1/groups/{ops}");
        let detail = h.get(&path, &admin).await;
        assert_eq!(
            (
                detail.body["managed_by"].as_str(),
                detail.body["member_count"].as_u64()
            ),
            (Some("ui"), Some(0))
        );
        let renamed = h
            .write(Method::PATCH, &path, &admin, json!({ "name": "ops-day" }))
            .await;
        assert_eq!(renamed.body["name"], "ops-day");
        let taken = h
            .write(Method::PATCH, &path, &admin, json!({ "name": "dev" }))
            .await;
        assert_eq!((taken.status.as_u16(), taken.code()), (409, "conflict"));
        let duplicate = h
            .write(
                Method::POST,
                "/admin/api/v1/groups",
                &admin,
                json!({ "name": "dev" }),
            )
            .await;
        assert_eq!(duplicate.status, 409);
        let blank = h
            .write(
                Method::POST,
                "/admin/api/v1/groups",
                &admin,
                json!({ "name": "" }),
            )
            .await;
        assert_eq!(blank.status, 422);

        assert_eq!(h.act(Method::DELETE, &path, &admin).await.status, 204);
        assert_eq!(h.get(&path, &admin).await.status, 404);
        assert_eq!(h.get("/admin/api/v1/groups/nope", &admin).await.status, 404);

        let rows = h.audit_rows().await;
        let shape: Vec<_> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.before.is_some(), r.after.is_some()))
            .collect();
        assert_eq!(
            shape,
            [
                ("group.create", false, true),
                ("group.create", false, true),
                ("group.create", false, true),
                ("group.update", true, true),
                ("group.delete", true, false),
            ]
        );
        assert_eq!(snapshot(rows[3].before.as_deref())["name"], "ops");
        assert_eq!(snapshot(rows[3].after.as_deref())["name"], "ops-day");
        assert_eq!(rows[3].target_id, ops.to_string());
    })
    .await;
}

#[tokio::test]
async fn members_are_added_listed_and_removed_idempotently() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let ops = create(&h, &admin, "ops").await;
        let bob = h.create_user("bob", Role::User).await;
        let carol = h.create_user("carol", Role::Admin).await;
        let members = format!("/admin/api/v1/groups/{ops}/members");
        for user in [bob, carol, bob] {
            let reply = h
                .write(Method::POST, &members, &admin, json!({ "user_id": user }))
                .await;
            assert_eq!(reply.status, 204, "{}", reply.body);
        }
        let listed = h.get(&format!("{members}?sort=-username"), &admin).await;
        let usernames: Vec<_> = listed.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["username"].as_str().unwrap())
            .collect();
        assert_eq!(usernames, ["carol", "bob"]);
        let admins = h.get(&format!("{members}?role=admin"), &admin).await;
        assert_eq!(admins.body["total"], 1);
        let count = h.get(&format!("/admin/api/v1/groups/{ops}"), &admin).await;
        assert_eq!(count.body["member_count"], 2);

        let unknown = h
            .write(
                Method::POST,
                &members,
                &admin,
                json!({ "user_id": 999_999 }),
            )
            .await;
        assert_eq!(unknown.status, 404);
        let no_group = h.get("/admin/api/v1/groups/999999/members", &admin).await;
        assert_eq!(no_group.status, 404);

        let remove = format!("{members}/{bob}");
        assert_eq!(h.act(Method::DELETE, &remove, &admin).await.status, 204);
        assert_eq!(h.act(Method::DELETE, &remove, &admin).await.status, 204);
        let after = h.get(&members, &admin).await;
        assert_eq!(after.body["total"], 1);

        let rows = h.audit_rows().await;
        let members_rows: Vec<_> = rows
            .iter()
            .filter(|r| r.action.starts_with("group.member"))
            .map(|r| (r.action.as_str(), r.before.is_some(), r.after.is_some()))
            .collect();
        assert_eq!(
            members_rows,
            [
                ("group.member_add", false, true),
                ("group.member_add", false, true),
                ("group.member_add", true, true),
                ("group.member_remove", true, false),
                ("group.member_remove", false, false),
            ]
        );
        let added = rows
            .iter()
            .find(|r| r.action == "group.member_add")
            .unwrap();
        assert_eq!(
            snapshot(added.after.as_deref()),
            json!({ "user_id": bob, "username": "bob" })
        );
    })
    .await;
}

#[tokio::test]
async fn config_managed_groups_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let now = time::now();
        let managed = group::ActiveModel {
            name: Set("from-config".to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Config),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
        .insert(&h.state.db)
        .await
        .unwrap();
        let bob = h.create_user("bob", Role::User).await;
        let path = format!("/admin/api/v1/groups/{}", managed.id);
        let detail = h.get(&path, &admin).await;
        assert_eq!(detail.body["managed_by"], "config");

        let attempts = [
            h.write(Method::PATCH, &path, &admin, json!({ "note": "x" }))
                .await,
            h.act(Method::DELETE, &path, &admin).await,
            h.write(
                Method::POST,
                &format!("{path}/members"),
                &admin,
                json!({ "user_id": bob }),
            )
            .await,
            h.act(Method::DELETE, &format!("{path}/members/{bob}"), &admin)
                .await,
        ];
        for reply in attempts {
            assert_eq!(
                (reply.status.as_u16(), reply.code()),
                (409, "managed_by_config")
            );
        }
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn plain_users_cannot_reach_group_routes() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let ops = create(&h, &admin, "ops").await;
        let user = h.console("usr", Role::User).await;
        let replies = [
            h.get("/admin/api/v1/groups", &user).await,
            h.get(&format!("/admin/api/v1/groups/{ops}"), &user).await,
            h.write(
                Method::POST,
                "/admin/api/v1/groups",
                &user,
                json!({ "name": "x" }),
            )
            .await,
            h.act(
                Method::DELETE,
                &format!("/admin/api/v1/groups/{ops}"),
                &user,
            )
            .await,
        ];
        for reply in replies {
            assert_eq!((reply.status.as_u16(), reply.code()), (403, "forbidden"));
        }
    })
    .await;
}
