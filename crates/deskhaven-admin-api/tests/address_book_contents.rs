//! Entries, tags and the device import of a shared book: passwords are write-only (never in
//! a response or an audit row), capacity, search, tag rename rewriting entries. Runs on
//! SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::devices::{register, report};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const BOOKS: &str = "/admin/api/v1/address-books";
const SECRET: &str = "correct-horse-battery";

async fn setup(db: &TestDb) -> (Harness, Console, String) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    let reply = h
        .write(Method::POST, BOOKS, &admin, json!({ "name": "ops" }))
        .await;
    let guid = reply.body["guid"].as_str().unwrap().to_owned();
    (h, admin, guid)
}

fn ids(reply: &Value) -> Vec<&str> {
    let items = reply["items"].as_array().unwrap();
    items.iter().map(|p| p["id"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn peers_are_added_updated_listed_and_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin, guid) = setup(db).await;
        let peers = format!("{BOOKS}/{guid}/peers");
        let add = |body: Value| h.write(Method::POST, &peers, &admin, body);

        let created = add(json!({
            "id": "100", "alias": "Front desk", "hostname": "pc-1", "tags": ["lobby"],
            "force_always_relay": true, "password": SECRET,
        }))
        .await;
        assert_eq!(created.status, 201, "{}", created.body);
        assert_eq!(created.body["alias"], "Front desk");
        assert_eq!(created.body["force_always_relay"], true);
        assert_eq!(created.body["has_password"], true);
        add(json!({ "id": "200", "note": "n", "username": "Carol" })).await;
        add(json!({ "id": "300" })).await;

        let duplicate = add(json!({ "id": "100" })).await;
        assert_eq!(
            (duplicate.status.as_u16(), duplicate.code()),
            (409, "conflict")
        );
        assert_eq!(add(json!({ "id": "" })).await.status, 422);
        assert_eq!(
            add(json!({ "id": "1", "tags": ["a".repeat(65)] }))
                .await
                .status,
            422
        );

        let all = h.get(&peers, &admin).await;
        assert_eq!(ids(&all.body), ["100", "200", "300"]);
        assert_eq!(all.body["items"][1]["has_password"], false);
        assert_eq!(
            ids(&h.get(&format!("{peers}?q=CAROL"), &admin).await.body),
            ["200"]
        );
        assert_eq!(
            h.get(&format!("{peers}?page=2&page_size=2"), &admin)
                .await
                .body["total"],
            3
        );
        assert_eq!(
            h.get(&format!("{BOOKS}/{guid}"), &admin).await.body["peer_count"],
            3
        );

        let one = format!("{peers}/100");
        let patched = h
            .write(
                Method::PATCH,
                &one,
                &admin,
                json!({ "alias": "Lobby", "tags": [] }),
            )
            .await;
        assert_eq!(patched.status, 200, "{}", patched.body);
        assert_eq!(
            (
                patched.body["alias"].as_str(),
                patched.body["has_password"].as_bool()
            ),
            (Some("Lobby"), Some(true))
        );
        assert_eq!(patched.body["hostname"], "pc-1", "absent fields stay");
        let missing = h
            .write(
                Method::PATCH,
                &format!("{peers}/nope"),
                &admin,
                json!({ "alias": "x" }),
            )
            .await;
        assert_eq!(missing.status, 404);

        let delete = format!("{peers}/delete");
        let removed = h
            .write(
                Method::POST,
                &delete,
                &admin,
                json!({ "ids": ["100", "ghost"] }),
            )
            .await;
        assert_eq!(removed.status, 204, "{}", removed.body);
        assert_eq!(ids(&h.get(&peers, &admin).await.body), ["200", "300"]);
        assert_eq!(
            h.write(Method::POST, &delete, &admin, json!({ "ids": [] }))
                .await
                .status,
            204
        );
    })
    .await;
}

#[tokio::test]
async fn passwords_are_write_only_everywhere() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin, guid) = setup(db).await;
        let peers = format!("{BOOKS}/{guid}/peers");
        let created = h
            .write(
                Method::POST,
                &peers,
                &admin,
                json!({ "id": "100", "password": SECRET }),
            )
            .await;
        let patched = h
            .write(
                Method::PATCH,
                &format!("{peers}/100"),
                &admin,
                json!({ "password": "new-secret-pw" }),
            )
            .await;
        let listed = h.get(&peers, &admin).await;

        for reply in [&created, &patched, &listed] {
            let text = reply.body.to_string();
            assert!(
                !text.contains(SECRET) && !text.contains("new-secret-pw"),
                "{text}"
            );
            assert!(!text.contains("\"password\""), "{text}");
        }
        let rows = h.audit_rows().await;
        let logged = serde_json::to_string(
            &rows
                .iter()
                .map(|r| (&r.before, &r.after))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(
            !logged.contains(SECRET) && !logged.contains("new-secret-pw"),
            "{logged}"
        );
        assert!(rows.iter().any(|r| r.action == "address_book.peer_add"));
        assert!(rows.iter().any(|r| r.action == "address_book.peer_update"));

        // The client gets it back (the console is write-only, not the client).
        let stored = h
            .books()
            .peers(
                deskhaven_core::addressbooks::Actor::Admin,
                &guid,
                common_page(),
            )
            .await
            .unwrap();
        let shown = format!("{:?}", stored.items[0].secret);
        assert!(!shown.contains("new-secret"), "Debug never prints it");

        let cleared = h
            .write(
                Method::PATCH,
                &format!("{peers}/100"),
                &admin,
                json!({ "password": "" }),
            )
            .await;
        assert_eq!(cleared.body["has_password"], false);
    })
    .await;
}

fn common_page() -> deskhaven_core::pagination::PageRequest {
    deskhaven_core::pagination::PageRequest {
        page: 1,
        page_size: 10,
    }
}

#[tokio::test]
async fn a_full_book_refuses_more_peers() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin, guid) = setup(db).await;
        h.set_setting("addressbook.max_peers", json!(1)).await;
        let peers = format!("{BOOKS}/{guid}/peers");
        h.write(Method::POST, &peers, &admin, json!({ "id": "1" }))
            .await;

        let full = h
            .write(Method::POST, &peers, &admin, json!({ "id": "2" }))
            .await;

        assert_eq!((full.status.as_u16(), full.code()), (409, "conflict"));
        assert_eq!(full.body["error"]["details"]["reason"], "book_full");
    })
    .await;
}

#[tokio::test]
async fn tags_are_added_recolored_renamed_and_deleted_with_their_entries() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin, guid) = setup(db).await;
        let tags = format!("{BOOKS}/{guid}/tags");
        let peers = format!("{BOOKS}/{guid}/peers");
        h.write(
            Method::POST,
            &peers,
            &admin,
            json!({ "id": "1", "tags": ["lobby", "keep"] }),
        )
        .await;

        let added = h
            .write(
                Method::POST,
                &tags,
                &admin,
                json!({ "name": "lobby", "color": 4_278_190_080_i64 }),
            )
            .await;
        assert_eq!(added.status, 201, "{}", added.body);
        h.write(Method::POST, &tags, &admin, json!({ "name": "keep" }))
            .await;
        let duplicate = h
            .write(Method::POST, &tags, &admin, json!({ "name": "lobby" }))
            .await;
        assert_eq!(duplicate.status, 409);
        assert_eq!(
            h.write(
                Method::POST,
                &tags,
                &admin,
                json!({ "name": "x", "color": -1 })
            )
            .await
            .status,
            422
        );
        assert_eq!(
            h.write(
                Method::POST,
                &tags,
                &admin,
                json!({ "name": "x", "color": 4_294_967_296_i64 })
            )
            .await
            .status,
            422
        );

        let listed = h.get(&tags, &admin).await;
        assert_eq!(
            listed.body["items"][1],
            json!({ "name": "lobby", "color": 4_278_190_080_i64 })
        );
        assert_eq!(
            h.get(&format!("{BOOKS}/{guid}"), &admin).await.body["tag_count"],
            2
        );

        let lobby = format!("{tags}/lobby");
        let both = h
            .write(
                Method::PATCH,
                &lobby,
                &admin,
                json!({ "name": "reception", "color": 7 }),
            )
            .await;
        assert_eq!(both.status, 200, "{}", both.body);
        assert_eq!(both.body, json!({ "name": "reception", "color": 7 }));
        let clash = h
            .write(
                Method::PATCH,
                &format!("{tags}/reception"),
                &admin,
                json!({ "name": "keep" }),
            )
            .await;
        assert_eq!(clash.status, 409);
        let none = h
            .write(
                Method::PATCH,
                &format!("{tags}/ghost"),
                &admin,
                json!({ "color": 1 }),
            )
            .await;
        assert_eq!(none.status, 404);
        assert_eq!(
            h.write(Method::PATCH, &lobby, &admin, json!({}))
                .await
                .status,
            422
        );
        let entry = h.get(&peers, &admin).await;
        assert_eq!(entry.body["items"][0]["tags"], json!(["reception", "keep"]));

        let delete = format!("{tags}/delete");
        let gone = h
            .write(
                Method::POST,
                &delete,
                &admin,
                json!({ "names": ["reception", "ghost"] }),
            )
            .await;
        assert_eq!(gone.status, 204);
        assert_eq!(
            h.get(&tags, &admin).await.body["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            h.get(&peers, &admin).await.body["items"][0]["tags"],
            json!(["keep"])
        );
    })
    .await;
}

#[tokio::test]
async fn devices_are_imported_as_entries() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin, guid) = setup(db).await;
        let win = register(&h, "100").await;
        report(&h, "100", "DESKTOP-1", "windows / Windows 10").await;
        let lin = register(&h, "200").await;
        let import = format!("{BOOKS}/{guid}/peers/import");

        let first = h
            .write(
                Method::POST,
                &import,
                &admin,
                json!({ "device_ids": [win, lin, 99_999] }),
            )
            .await;
        assert_eq!(first.status, 200, "{}", first.body);
        assert_eq!(
            first.body,
            json!({ "added": ["100", "200"], "existing": [], "missing": [99_999] })
        );
        let again = h
            .write(
                Method::POST,
                &import,
                &admin,
                json!({ "device_ids": [win] }),
            )
            .await;
        assert_eq!(
            again.body,
            json!({ "added": [], "existing": ["100"], "missing": [] })
        );
        let entry = h
            .get(&format!("{BOOKS}/{guid}/peers?q=desktop"), &admin)
            .await;
        assert_eq!(entry.body["items"][0]["platform"], "Windows");
        assert_eq!(entry.body["items"][0]["hostname"], "DESKTOP-1");
        assert_eq!(
            h.write(Method::POST, &import, &admin, json!({ "device_ids": [] }))
                .await
                .status,
            422
        );
        let rows = h.audit_rows().await;
        let imports = rows
            .iter()
            .filter(|r| r.action == "address_book.peer_import")
            .count();
        assert_eq!(imports, 1, "an import that adds nothing records nothing");
    })
    .await;
}
