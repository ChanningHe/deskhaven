//! The M3-06 acceptance: what the console changes in a shared book is what the client API
//! reads, and what a client writes is what the console shows. Both sides run on one state
//! (the real routers and middleware stacks). Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::client::{client_call, client_token};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::groups::NewGroup;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const BOOKS: &str = "/admin/api/v1/address-books";
const SECRET: &str = "correct-horse-battery";

struct Scene {
    h: Harness,
    admin: Console,
    guid: String,
    alice: String,
    alice_id: i64,
    bob: String,
}

/// Book "ops" with entry 100 (password) and tag lobby; alice (rule 2) and, through a group,
/// bob (rule 1) are granted it.
async fn scene(db: &TestDb) -> Scene {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    let alice_id = h.create_user("alice", Role::User).await;
    let bob_id = h.create_user("bob", Role::User).await;
    let team = h
        .state
        .groups
        .create(NewGroup::new("team"))
        .await
        .unwrap()
        .id;
    h.state.groups.add_member(team, bob_id).await.unwrap();
    let created = h
        .write(
            Method::POST,
            BOOKS,
            &admin,
            json!({ "name": "ops", "note": "shared" }),
        )
        .await;
    let guid = created.body["guid"].as_str().unwrap().to_owned();
    let book = format!("{BOOKS}/{guid}");
    let shares = [
        json!({ "subject_type": "user", "subject_id": alice_id, "rule": 2 }),
        json!({ "subject_type": "group", "subject_id": team, "rule": 1 }),
    ];
    for share in shares {
        let granted = h
            .write(Method::PUT, &format!("{book}/shares"), &admin, share)
            .await;
        assert_eq!(granted.status, 204, "{}", granted.body);
    }
    let peer = json!({ "id": "100", "alias": "Front desk", "tags": ["lobby"], "password": SECRET });
    h.write(Method::POST, &format!("{book}/peers"), &admin, peer)
        .await;
    let tag = json!({ "name": "lobby", "color": 4_278_190_080_i64 });
    h.write(Method::POST, &format!("{book}/tags"), &admin, tag)
        .await;
    let alice = client_token(&h, "alice").await;
    let bob = client_token(&h, "bob").await;
    Scene {
        h,
        admin,
        guid,
        alice,
        alice_id,
        bob,
    }
}

async fn read(s: &Scene, token: &str, path: &str) -> (StatusCode, Value) {
    client_call(&s.h, Method::POST, path, token, None).await
}

fn peers_path(guid: &str) -> String {
    format!("/api/ab/peers?current=1&pageSize=100&ab={guid}")
}

#[tokio::test]
async fn what_the_console_sets_up_is_what_the_clients_read() {
    each_migrated_backend(async |db: &TestDb| {
        let s = scene(db).await;

        let (status, profiles) = read(
            &s,
            &s.alice,
            "/api/ab/shared/profiles?current=1&pageSize=100",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(profiles["total"], 1);
        let profile = &profiles["data"][0];
        assert_eq!(profile["guid"], s.guid.as_str());
        assert_eq!(
            (
                profile["name"].as_str(),
                profile["note"].as_str(),
                profile["owner"].as_str()
            ),
            (Some("ops"), Some("shared"), Some(""))
        );
        assert_eq!(profile["rule"], 2);
        let (_, bobs) = read(&s, &s.bob, "/api/ab/shared/profiles?current=1&pageSize=100").await;
        assert_eq!(
            bobs["data"][0]["rule"], 1,
            "a group grant reaches its members"
        );

        for token in [&s.alice, &s.bob] {
            let (status, peers) = read(&s, token, &peers_path(&s.guid)).await;
            assert_eq!(status, StatusCode::OK);
            let peer = &peers["data"][0];
            assert_eq!(
                (peer["id"].as_str(), peer["alias"].as_str()),
                (Some("100"), Some("Front desk"))
            );
            assert_eq!(peer["tags"], json!(["lobby"]));
            // The client connects with it: a read grant hands out the password (06 4.2).
            assert_eq!(peer["password"], SECRET);
            let (_, tags) = read(&s, token, &format!("/api/ab/tags/{}", s.guid)).await;
            assert_eq!(
                tags,
                json!([{ "name": "lobby", "color": 4_278_190_080_i64 }])
            );
        }
    })
    .await;
}

#[tokio::test]
async fn console_changes_show_up_for_the_clients_on_their_next_read() {
    each_migrated_backend(async |db: &TestDb| {
        let s = scene(db).await;
        let patch =
            async |path: String, body: Value| s.h.write(Method::PATCH, &path, &s.admin, body).await;
        let book = format!("{BOOKS}/{}", s.guid);

        patch(
            book.clone(),
            json!({ "name": "operations", "note": "moved" }),
        )
        .await;
        patch(
            format!("{book}/peers/100"),
            json!({ "alias": "Lobby PC", "password": "" }),
        )
        .await;
        patch(
            format!("{book}/tags/lobby"),
            json!({ "name": "reception", "color": 9 }),
        )
        .await;
        s.h.write(
            Method::POST,
            &format!("{book}/peers"),
            &s.admin,
            json!({ "id": "200" }),
        )
        .await;

        let (_, profiles) = read(
            &s,
            &s.alice,
            "/api/ab/shared/profiles?current=1&pageSize=100",
        )
        .await;
        assert_eq!(profiles["data"][0]["name"], "operations");
        assert_eq!(profiles["data"][0]["note"], "moved");
        let (_, peers) = read(&s, &s.alice, &peers_path(&s.guid)).await;
        assert_eq!(peers["total"], 2);
        assert_eq!(peers["data"][0]["alias"], "Lobby PC");
        assert_eq!(peers["data"][0]["password"], "");
        assert_eq!(peers["data"][0]["tags"], json!(["reception"]));
        let (_, tags) = read(&s, &s.alice, &format!("/api/ab/tags/{}", s.guid)).await;
        assert_eq!(tags, json!([{ "name": "reception", "color": 9 }]));

        // Revoking a grant takes the book away from that user only.
        let revoke = format!("{book}/shares/user/{}", s.alice_id);
        assert_eq!(s.h.act(Method::DELETE, &revoke, &s.admin).await.status, 204);
        let (gone, _) = read(&s, &s.alice, &peers_path(&s.guid)).await;
        assert_eq!(gone, StatusCode::NOT_FOUND);
        let (_, profiles) = read(
            &s,
            &s.alice,
            "/api/ab/shared/profiles?current=1&pageSize=100",
        )
        .await;
        assert_eq!(profiles["total"], 0);
        let (still, _) = read(&s, &s.bob, &peers_path(&s.guid)).await;
        assert_eq!(still, StatusCode::OK);

        assert_eq!(s.h.act(Method::DELETE, &book, &s.admin).await.status, 204);
        let (deleted, _) = read(&s, &s.bob, &peers_path(&s.guid)).await;
        assert_eq!(deleted, StatusCode::NOT_FOUND);
    })
    .await;
}

#[tokio::test]
async fn client_writes_show_up_in_the_console_and_rule_1_cannot_write() {
    each_migrated_backend(async |db: &TestDb| {
        let s = scene(db).await;
        let book = format!("{BOOKS}/{}", s.guid);
        let add = format!("/api/ab/peer/add/{}", s.guid);
        let body =
            Some(json!({ "id": "300", "alias": "from client", "password": "client-set-pw" }));

        let (written, _) = client_call(&s.h, Method::POST, &add, &s.alice, body.clone()).await;
        let (denied, _) = client_call(&s.h, Method::POST, &add, &s.bob, body).await;

        assert_eq!(written, StatusCode::OK);
        assert_eq!(denied, StatusCode::FORBIDDEN);
        let listed = s.h.get(&format!("{book}/peers"), &s.admin).await;
        let added = &listed.body["items"][1];
        assert_eq!(
            (
                added["id"].as_str(),
                added["alias"].as_str(),
                added["has_password"].as_bool()
            ),
            (Some("300"), Some("from client"), Some(true))
        );
        assert!(!listed.body.to_string().contains("client-set-pw"));
        assert_eq!(s.h.get(&book, &s.admin).await.body["peer_count"], 2);
    })
    .await;
}
