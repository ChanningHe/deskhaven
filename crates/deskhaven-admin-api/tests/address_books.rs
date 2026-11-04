//! Shared address book endpoints: list with counts, create, detail, update, delete, grants
//! (user / group x rule 1-3), audit rows with before / after. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::groups::NewGroup;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const BOOKS: &str = "/admin/api/v1/address-books";

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

async fn create(h: &Harness, console: &Console, body: Value) -> String {
    let reply = h.write(Method::POST, BOOKS, console, body).await;
    assert_eq!(reply.status, 201, "{}", reply.body);
    reply.body["guid"].as_str().unwrap().to_owned()
}

fn names(reply: &Value) -> Vec<&str> {
    let items = reply["items"].as_array().unwrap();
    items.iter().map(|b| b["name"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn books_are_created_listed_updated_and_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let alice = h.create_user("alice", Role::User).await;
        let ops = create(&h, &admin, json!({ "name": "ops", "note": "n" })).await;
        create(&h, &admin, json!({ "name": "lab", "owner_user_id": alice })).await;
        create(&h, &admin, json!({ "name": "zoo" })).await;

        let listed = h.get(&format!("{BOOKS}?sort=-name"), &admin).await;
        assert_eq!(names(&listed.body), ["zoo", "ops", "lab"]);
        assert_eq!(listed.body["total"], 3);
        let lab = &listed.body["items"][2];
        assert_eq!(
            (lab["owner"].as_str(), lab["owner_user_id"].as_i64()),
            (Some("alice"), Some(alice))
        );
        let found = h.get(&format!("{BOOKS}?q=O"), &admin).await;
        assert_eq!(names(&found.body), ["ops", "zoo"]);
        let paged = h.get(&format!("{BOOKS}?page=2&page_size=2"), &admin).await;
        assert_eq!(
            (paged.body["total"].as_u64(), names(&paged.body)),
            (Some(3), vec!["zoo"])
        );
        let bad = h.get(&format!("{BOOKS}?sort=peer_count"), &admin).await;
        assert_eq!(bad.status, 422);

        let book = format!("{BOOKS}/{ops}");
        let detail = h.get(&book, &admin).await;
        assert_eq!(detail.body["note"], "n");
        assert_eq!(detail.body["peer_count"], 0);
        let renamed = h
            .write(
                Method::PATCH,
                &book,
                &admin,
                json!({ "name": "operations", "note": "m" }),
            )
            .await;
        assert_eq!(
            (renamed.body["name"].as_str(), renamed.body["note"].as_str()),
            (Some("operations"), Some("m"))
        );
        assert_eq!(h.act(Method::DELETE, &book, &admin).await.status, 204);
        assert_eq!(h.get(&book, &admin).await.status, 404);
        assert_eq!(h.act(Method::DELETE, &book, &admin).await.status, 404);

        let rows = h.audit_rows().await;
        let shape: Vec<_> = rows
            .iter()
            .filter(|r| r.target_type == "address_book")
            .map(|r| (r.action.as_str(), r.before.is_some(), r.after.is_some()))
            .collect();
        assert_eq!(
            shape,
            [
                ("address_book.create", false, true),
                ("address_book.create", false, true),
                ("address_book.create", false, true),
                ("address_book.update", true, true),
                ("address_book.delete", true, false),
            ]
        );
        assert_eq!(rows[0].target_id, ops);
    })
    .await;
}

#[tokio::test]
async fn invalid_names_owners_and_empty_patches_are_refused_or_ignored() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let guid = create(&h, &admin, json!({ "name": "Ops" })).await;
        let post = |body: Value| h.write(Method::POST, BOOKS, &admin, body);

        let duplicate = post(json!({ "name": "OPS" })).await;
        assert_eq!(
            (duplicate.status.as_u16(), duplicate.code()),
            (409, "conflict")
        );
        let reserved = post(json!({ "name": "my address book" })).await;
        assert_eq!(reserved.status, 409);
        assert_eq!(post(json!({ "name": "  " })).await.status, 422);
        assert_eq!(post(json!({ "name": "x".repeat(65) })).await.status, 422);
        assert_eq!(
            post(json!({ "name": "y", "owner_user_id": 999_999 }))
                .await
                .status,
            404
        );
        assert_eq!(
            post(json!({ "name": "y", "surprise": 1 })).await.status,
            422
        );

        let audits = h.audit_rows().await.len();
        let book = format!("{BOOKS}/{guid}");
        let before = h.get(&book, &admin).await.body;
        let same = h.write(Method::PATCH, &book, &admin, json!({})).await;
        assert_eq!(same.status, 200, "{}", same.body);
        assert_eq!(same.body["updated_at"], before["updated_at"]);
        assert_eq!(
            h.audit_rows().await.len(),
            audits,
            "nothing changed, nothing recorded"
        );
    })
    .await;
}

#[tokio::test]
async fn personal_books_are_not_console_books() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let alice = h.create_user("alice", Role::User).await;
        let personal = h.books().personal(alice).await.unwrap();
        create(&h, &admin, json!({ "name": "ops" })).await;

        let listed = h.get(BOOKS, &admin).await;
        assert_eq!(names(&listed.body), ["ops"]);
        let book = format!("{BOOKS}/{}", personal.guid);
        for path in [
            book.clone(),
            format!("{book}/peers"),
            format!("{book}/tags"),
            format!("{book}/shares"),
        ] {
            assert_eq!(h.get(&path, &admin).await.status, 404, "{path}");
        }
        let patch = h
            .write(Method::PATCH, &book, &admin, json!({ "name": "x" }))
            .await;
        assert_eq!(patch.status, 404);
        assert_eq!(h.act(Method::DELETE, &book, &admin).await.status, 404);
        let peer = h
            .write(
                Method::POST,
                &format!("{book}/peers"),
                &admin,
                json!({ "id": "1" }),
            )
            .await;
        assert_eq!(peer.status, 404);
    })
    .await;
}

#[tokio::test]
async fn grants_are_set_listed_changed_and_revoked() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let alice = h.create_user("alice", Role::User).await;
        let team = h
            .state
            .groups
            .create(NewGroup::new("team"))
            .await
            .unwrap()
            .id;
        let guid = create(&h, &admin, json!({ "name": "ops" })).await;
        let shares = format!("{BOOKS}/{guid}/shares");
        let put = |body: Value| h.write(Method::PUT, &shares, &admin, body);

        assert_eq!(
            put(json!({ "subject_type": "user", "subject_id": alice, "rule": 2 }))
                .await
                .status,
            204
        );
        assert_eq!(
            put(json!({ "subject_type": "group", "subject_id": team, "rule": 1 }))
                .await
                .status,
            204
        );
        let listed = h.get(&shares, &admin).await;
        assert_eq!(
            listed.body["items"],
            json!([
                { "subject_type": "group", "subject_id": team, "subject_name": "team", "rule": 1 },
                { "subject_type": "user", "subject_id": alice, "subject_name": "alice", "rule": 2 },
            ])
        );
        assert_eq!(
            put(json!({ "subject_type": "user", "subject_id": alice, "rule": 3 }))
                .await
                .status,
            204
        );
        assert_eq!(h.get(&shares, &admin).await.body["items"][1]["rule"], 3);
        assert_eq!(
            h.get(&format!("{BOOKS}/{guid}"), &admin).await.body["share_count"],
            2
        );

        let ghost = put(json!({ "subject_type": "user", "subject_id": 999_999, "rule": 1 })).await;
        assert_eq!(ghost.status, 404);
        assert_eq!(
            put(json!({ "subject_type": "user", "subject_id": alice, "rule": 4 }))
                .await
                .status,
            422
        );
        assert_eq!(
            put(json!({ "subject_type": "robot", "subject_id": alice, "rule": 1 }))
                .await
                .status,
            422
        );

        let revoke = format!("{shares}/user/{alice}");
        assert_eq!(h.act(Method::DELETE, &revoke, &admin).await.status, 204);
        assert_eq!(
            h.act(Method::DELETE, &revoke, &admin).await.status,
            204,
            "idempotent"
        );
        assert_eq!(
            h.act(Method::DELETE, &format!("{shares}/robot/1"), &admin)
                .await
                .status,
            404
        );
        assert_eq!(
            h.get(&shares, &admin).await.body["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );

        let rows = h.audit_rows().await;
        let shape: Vec<_> = rows
            .iter()
            .filter(|r| r.action.starts_with("address_book.share"))
            .map(|r| {
                (
                    r.action.as_str(),
                    r.before.is_some(),
                    r.after.is_some(),
                    r.target_id.as_str(),
                )
            })
            .collect();
        let g = guid.as_str();
        assert_eq!(
            shape,
            [
                ("address_book.share_set", false, true, g),
                ("address_book.share_set", false, true, g),
                ("address_book.share_set", true, true, g),
                ("address_book.share_remove", true, false, g),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_or_group_drops_their_grants_from_the_list() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let alice = h.create_user("alice", Role::User).await;
        let guid = create(&h, &admin, json!({ "name": "ops" })).await;
        let shares = format!("{BOOKS}/{guid}/shares");
        let body = json!({ "subject_type": "user", "subject_id": alice, "rule": 1 });
        h.write(Method::PUT, &shares, &admin, body).await;

        h.state.users.delete(alice).await.unwrap();

        assert_eq!(h.get(&shares, &admin).await.body["items"], json!([]));
    })
    .await;
}
