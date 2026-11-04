//! "My account" (M3-13), what the user owns or sees: their devices with online state and
//! their address books with the effective rule; plus the gate of every self-service route
//! (no session, client session, missing CSRF, unfinished account setup). Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::devices::{heartbeat, register, report};
use common::own::{ACCOUNT, BOOKS, DEVICES, DISABLE, ME, RECOVERY, REVOKE_OTHERS, SESSIONS};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::addressbooks::{Actor, NewSharedBook, Rule, Subject};
use deskhaven_core::users::{ProfileUpdate, Role};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

async fn own_device(h: &Harness, admin: &Console, rustdesk_id: &str, owner: Option<i64>) {
    let id = register(h, rustdesk_id).await;
    report(h, rustdesk_id, &format!("host-{rustdesk_id}"), "Linux").await;
    let path = format!("/admin/api/v1/devices/{id}");
    let body = json!({ "owner_user_id": owner });
    let done = h.write(Method::PATCH, &path, admin, body).await;
    assert_eq!(done.status, 200, "{:?}", done.body);
}

fn device_ids(reply: &Value) -> Vec<&str> {
    let items = reply["items"].as_array().unwrap();
    let mut ids: Vec<&str> = items
        .iter()
        .map(|d| d["rustdesk_id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    ids
}

#[tokio::test]
async fn my_devices_are_the_owned_ones_with_their_online_state() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;
        let alice_id = h.user("alice", Role::User).await;
        let bob_id = h.user("bob", Role::User).await;
        let alice = h.signed_in("alice").await;
        own_device(&h, &admin, "100000001", Some(alice_id)).await;
        own_device(&h, &admin, "100000002", Some(alice_id)).await;
        own_device(&h, &admin, "200000001", Some(bob_id)).await;
        own_device(&h, &admin, "300000001", None).await;
        heartbeat(&h, "100000001", &[]);
        let b = db.backend;

        let mine = h.get(DEVICES, &alice).await;
        let spoofed = h.get(&format!("{DEVICES}?owner={bob_id}"), &alice).await;
        let paged = h
            .get(&format!("{DEVICES}?page=2&page_size=10"), &alice)
            .await;

        assert_eq!(mine.status, 200, "{b}: {:?}", mine.body);
        assert_eq!(device_ids(&mine.body), ["100000001", "100000002"], "{b}");
        assert_eq!(mine.body["total"], 2, "{b}");
        let online: Vec<_> = mine.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["rustdesk_id"].as_str().unwrap(),
                    d["online"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(online, [("100000001", true), ("100000002", false)], "{b}");
        let first = &mine.body["items"][0];
        assert_eq!(first["hostname"], "host-100000001", "{b}");
        assert_eq!(first["os"], "Linux", "{b}");
        assert!(
            first.get("uuid").is_none() && first.get("last_ip").is_none(),
            "{b}"
        );
        // A client-supplied owner is not a filter here: whatever the status, nothing of
        // bob's comes back.
        assert!(!spoofed.body.to_string().contains("200000001"), "{b}");
        assert_eq!(paged.body["items"], json!([]), "{b}");
        assert_eq!(paged.body["total"], 2, "{b}");
    })
    .await;
}

#[tokio::test]
async fn my_address_books_are_the_personal_book_and_the_shared_ones_with_my_rule() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let alice_id = h.user("alice", Role::User).await;
        let bob_id = h.user("bob", Role::User).await;
        let alice = h.signed_in("alice").await;
        let books = h.books();
        let book = |name: &str, owner| NewSharedBook {
            name: name.to_owned(),
            note: String::new(),
            owner_user_id: owner,
        };
        books.personal(bob_id).await.unwrap();
        books
            .create_shared(book("alice-team", Some(alice_id)))
            .await
            .unwrap();
        let shared = books
            .create_shared(book("bob-readonly", Some(bob_id)))
            .await
            .unwrap();
        books
            .set_share(
                Actor::Admin,
                &shared.guid,
                Subject::User(alice_id),
                Rule::ReadOnly,
            )
            .await
            .unwrap();
        books
            .create_shared(book("bob-private", Some(bob_id)))
            .await
            .unwrap();
        let b = db.backend;

        let mine = h.get(BOOKS, &alice).await;

        assert_eq!(mine.status, 200, "{b}: {:?}", mine.body);
        let seen: Vec<(String, String, i64)> = mine.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                (
                    x["kind"].as_str().unwrap().to_owned(),
                    x["name"].as_str().unwrap().to_owned(),
                    x["rule"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(seen.len(), 3, "{b}: {seen:?}");
        assert_eq!(seen[0].0, "personal", "{b}");
        assert_eq!(seen[0].2, 3, "{b}");
        assert_eq!(
            seen[1..],
            [
                ("shared".to_owned(), "alice-team".to_owned(), 3),
                ("shared".to_owned(), "bob-readonly".to_owned(), 1),
            ],
            "{b}"
        );
        assert_eq!(mine.body["shared_total"], 2, "{b}");
        let readonly = &mine.body["items"][2];
        assert_eq!(readonly["owner"], "bob", "{b}");
        assert_eq!(readonly["guid"], shared.guid, "{b}");
        assert!(!mine.body.to_string().contains("bob-private"), "{b}");
    })
    .await;
}

struct Route {
    method: Method,
    path: &'static str,
    body: Option<Value>,
}

fn routes() -> Vec<Route> {
    let route = |method, path, body| Route { method, path, body };
    let password = || Some(json!({ "current_password": "x" }));
    vec![
        route(Method::GET, ACCOUNT, None),
        route(Method::PATCH, ME, Some(json!({ "display_name": "x" }))),
        route(
            Method::POST,
            DISABLE,
            Some(json!({ "current_password": "x", "code": "123456" })),
        ),
        route(Method::POST, RECOVERY, password()),
        route(Method::GET, SESSIONS, None),
        route(Method::DELETE, "/admin/api/v1/me/sessions/1", None),
        route(Method::POST, REVOKE_OTHERS, None),
        route(Method::GET, DEVICES, None),
        route(Method::GET, BOOKS, None),
    ]
}

#[tokio::test]
async fn every_self_service_route_needs_a_console_session_and_a_finished_setup() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.user("alice", Role::User).await;
        let restricted_id = h.user("carol", Role::User).await;
        let update = ProfileUpdate {
            must_change_password: Some(true),
            ..ProfileUpdate::default()
        };
        h.state
            .users
            .update_profile(restricted_id, update)
            .await
            .unwrap();
        let alice = h.signed_in("alice").await;
        let carol = h.signed_in("carol").await;
        // A client session presented as the console cookie, with the CSRF value derived
        // from its token like a console session's.
        let cookie = h.client_session_cookie("dave").await;
        let token = cookie.split_once('=').unwrap().1.to_owned();
        let dave = Console {
            user_id: 0,
            csrf: deskhaven_admin_api::csrf_token(&token),
            cookie,
        };
        let b = db.backend;

        for r in routes() {
            let name = format!("{b}: {} {}", r.method, r.path);
            let anonymous = h
                .as_anonymous(r.method.clone(), r.path, r.body.clone())
                .await;
            let client = h
                .as_console(r.method.clone(), r.path, &dave, r.body.clone())
                .await;
            let restricted = h
                .as_console(r.method.clone(), r.path, &carol, r.body.clone())
                .await;
            assert_eq!(anonymous.status, 401, "{name}");
            assert_eq!(
                client.status, 401,
                "{name}: a client session is no console session"
            );
            assert_eq!(restricted.status, 403, "{name}");
            assert_eq!(restricted.code(), "setup_required", "{name}");
            if r.method != Method::GET {
                let no_csrf = h
                    .send(
                        r.method.clone(),
                        r.path,
                        Some(&alice.cookie),
                        None,
                        r.body.clone(),
                    )
                    .await;
                assert_eq!(no_csrf.status, 403, "{name}");
                assert_eq!(no_csrf.code(), "csrf_failed", "{name}");
            }
        }
    })
    .await;
}
