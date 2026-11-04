//! Who may use the address book endpoints: admins and owners only. Plain users get 403 and
//! change nothing, no session gets 401, a write without the CSRF header 403. A client
//! session is no console session. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::addressbooks::{Actor, NewSharedBook};
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

struct Call {
    method: Method,
    path: String,
    body: Option<Value>,
}

fn calls(guid: &str, user: i64) -> Vec<Call> {
    let book = format!("/admin/api/v1/address-books/{guid}");
    let call = |method, path: String, body| Call { method, path, body };
    vec![
        call(Method::GET, "/admin/api/v1/address-books".into(), None),
        call(
            Method::POST,
            "/admin/api/v1/address-books".into(),
            Some(json!({ "name": "new" })),
        ),
        call(Method::GET, book.clone(), None),
        call(Method::PATCH, book.clone(), Some(json!({ "note": "x" }))),
        call(Method::GET, format!("{book}/shares"), None),
        call(
            Method::PUT,
            format!("{book}/shares"),
            Some(json!({ "subject_type": "user", "subject_id": user, "rule": 3 })),
        ),
        call(Method::DELETE, format!("{book}/shares/user/{user}"), None),
        call(Method::GET, format!("{book}/peers"), None),
        call(
            Method::POST,
            format!("{book}/peers"),
            Some(json!({ "id": "9" })),
        ),
        call(
            Method::PATCH,
            format!("{book}/peers/1"),
            Some(json!({ "alias": "x" })),
        ),
        call(
            Method::POST,
            format!("{book}/peers/delete"),
            Some(json!({ "ids": ["1"] })),
        ),
        call(
            Method::POST,
            format!("{book}/peers/import"),
            Some(json!({ "device_ids": [1] })),
        ),
        call(Method::GET, format!("{book}/tags"), None),
        call(
            Method::POST,
            format!("{book}/tags"),
            Some(json!({ "name": "t" })),
        ),
        call(
            Method::PATCH,
            format!("{book}/tags/t"),
            Some(json!({ "color": 1 })),
        ),
        call(
            Method::POST,
            format!("{book}/tags/delete"),
            Some(json!({ "names": ["t"] })),
        ),
        call(Method::DELETE, book, None),
    ]
}

async fn send(h: &Harness, call: &Call, who: Option<&Console>, csrf: bool) -> (u16, String) {
    let cookie = who.map(|c| c.cookie.as_str());
    // Without a session any non-empty CSRF value passes, so the 401 shows.
    let token = match who {
        Some(console) => csrf.then_some(console.csrf.as_str()),
        None => Some("any"),
    };
    let reply = h
        .send(
            call.method.clone(),
            &call.path,
            cookie,
            token,
            call.body.clone(),
        )
        .await;
    (reply.status.as_u16(), reply.code().to_owned())
}

#[tokio::test]
async fn plain_users_clients_and_strangers_are_refused_everywhere() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let user = h.console("plain", Role::User).await;
        let admin = h.console("adm", Role::Admin).await;
        let client = h.client_session_cookie("client").await;
        let book = h
            .books()
            .create_shared(NewSharedBook {
                name: "ops".into(),
                note: String::new(),
                owner_user_id: None,
            })
            .await
            .unwrap();

        for call in calls(&book.guid, user.user_id) {
            let cell = format!("{} {}", call.method, call.path);
            assert_eq!(
                send(&h, &call, Some(&user), true).await,
                (403, "forbidden".into()),
                "{cell}"
            );
            assert_eq!(
                send(&h, &call, None, true).await,
                (401, "unauthorized".into()),
                "{cell}"
            );
            if call.method == Method::GET {
                let as_client = h
                    .send(call.method.clone(), &call.path, Some(&client), None, None)
                    .await;
                assert_eq!(as_client.status, 401, "{cell}");
            }
            if call.method != Method::GET {
                let got = send(&h, &call, Some(&admin), false).await;
                assert_eq!(got, (403, "csrf_failed".into()), "{cell}");
            }
        }
        assert!(
            h.audit_rows().await.is_empty(),
            "refused calls leave no audit row"
        );
        let unchanged = h.books().admin_book(&book.guid).await.unwrap();
        assert_eq!(
            (unchanged.book.name.as_str(), unchanged.book.note.as_str()),
            ("ops", "")
        );
        assert!(
            h.books()
                .peers(Actor::Admin, &book.guid, Default::default())
                .await
                .unwrap()
                .items
                .is_empty()
        );
    })
    .await;
}

#[tokio::test]
async fn admins_and_owners_reach_the_read_endpoints() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;
        let owner = h.console("own", Role::Owner).await;
        let book = h
            .books()
            .create_shared(NewSharedBook {
                name: "ops".into(),
                note: String::new(),
                owner_user_id: None,
            })
            .await
            .unwrap();

        for who in [&admin, &owner] {
            for call in calls(&book.guid, who.user_id)
                .iter()
                .filter(|c| c.method == Method::GET)
            {
                let (status, _) = send(&h, call, Some(who), true).await;
                assert_eq!(status, 200, "{} {}", call.method, call.path);
            }
        }
    })
    .await;
}
