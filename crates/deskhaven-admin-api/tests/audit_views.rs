//! Audit views (M3-08), part one: permissions, the connection list, the time window, the
//! retired-session and deleted-controller cases, and query validation. Admin only. Runs on
//! SQLite and PostgreSQL. File, alarm, login and console lists: `audit_views_kinds.rs`; the
//! 100k-row timing: `audit_views_perf.rs`.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::audit_rows::{self as rows, AUDIT, KINDS, ids, list, secs, setup};
use deskhaven_core::audit::RETIRED_PREFIX;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use serde_json::json;

#[tokio::test]
async fn only_admins_read_the_audit_views() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let user = h.console("bob", Role::User).await;
        let owner = h.console("own", Role::Owner).await;
        let client_session = h.client_session_cookie("carol").await;

        for kind in KINDS {
            let path = format!("{AUDIT}/{kind}");
            let denied = h.get(&path, &user).await;
            assert_eq!((denied.status.as_u16(), denied.code()), (403, "forbidden"));
            let anonymous = h.send(axum::http::Method::GET, &path, None, None, None);
            assert_eq!(anonymous.await.status, 401, "{kind}");
            let client = h
                .send(
                    axum::http::Method::GET,
                    &path,
                    Some(&client_session),
                    None,
                    None,
                )
                .await;
            assert_eq!(client.status, 401, "{kind}");
            assert_eq!(h.get(&path, &admin).await.status, 200, "{kind}");
            assert_eq!(h.get(&path, &owner).await.status, 200, "{kind}");
        }
    })
    .await;
}

#[tokio::test]
async fn connections_list_newest_first_with_filters_and_pages() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        let first = rows::connection(&h, "100", 1, 10, |r| {
            r.controller_user_id = Set(Some(bob));
            r.conn_type = Set(Some(0));
            r.peer_name = Set(Some("Reception PC".to_owned()));
        })
        .await;
        let second = rows::connection(&h, "100", 2, 20, |r| r.conn_type = Set(Some(1))).await;
        let third = rows::connection(&h, "200", 1, 30, |r| r.conn_type = Set(Some(0))).await;

        let all = list(&h, &admin, "connections", "").await;
        assert_eq!(ids(&all), [third, second, first]);
        assert_eq!(all["total"], 3);
        assert_eq!(
            (all["page"].as_u64(), all["page_size"].as_u64()),
            (Some(1), Some(20))
        );
        let oldest = list(&h, &admin, "connections", "sort=created_at").await;
        assert_eq!(ids(&oldest), [first, second, third]);

        let device = list(&h, &admin, "connections", "device=100").await;
        assert_eq!(ids(&device), [second, first]);
        let user = list(&h, &admin, "connections", &format!("user={bob}")).await;
        assert_eq!(ids(&user), [first]);
        let kind = list(&h, &admin, "connections", "conn_type=0").await;
        assert_eq!(ids(&kind), [third, first]);
        let text = list(&h, &admin, "connections", "q=reception").await;
        assert_eq!(ids(&text), [first]);
        let literal = list(&h, &admin, "connections", "q=%25").await;
        assert_eq!(literal["total"], 0);

        let page = list(&h, &admin, "connections", "page=2&page_size=2").await;
        assert_eq!((ids(&page), page["total"].as_u64()), (vec![first], Some(3)));
    })
    .await;
}

#[tokio::test]
async fn time_range_includes_from_and_excludes_to() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let a = rows::connection(&h, "100", 1, 10, |_| {}).await;
        let b = rows::connection(&h, "100", 2, 20, |_| {}).await;
        let c = rows::connection(&h, "100", 3, 30, |_| {}).await;

        let from = list(&h, &admin, "connections", &format!("from={}", secs(20))).await;
        assert_eq!(ids(&from), [c, b]);
        let to = list(&h, &admin, "connections", &format!("to={}", secs(20))).await;
        assert_eq!(ids(&to), [a]);
        let both = format!("from={}&to={}", secs(10), secs(30));
        assert_eq!(ids(&list(&h, &admin, "connections", &both).await), [b, a]);

        let inverted = format!("from={}&to={}", secs(30), secs(10));
        let bad = h
            .get(&format!("{AUDIT}/connections?{inverted}"), &admin)
            .await;
        assert_eq!(
            (bad.status.as_u16(), bad.code()),
            (422, "validation_failed")
        );
    })
    .await;
}

#[tokio::test]
async fn connection_view_hides_retired_session_ids_and_names_the_controller() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        rows::connection(&h, "100", 1, 10, |r| {
            r.session_id = Set(format!("{RETIRED_PREFIX}41"));
        })
        .await;
        rows::connection(&h, "100", 2, 20, |r| {
            r.controller_user_id = Set(Some(bob));
            r.peer_rustdesk_id = Set(Some("900".to_owned()));
            r.peer_name = Set(Some("<img src=x onerror=alert(1)>".to_owned()));
            r.conn_type = Set(Some(4));
            r.primary_auth = Set(Some(2));
            r.two_factor = Set(Some(1));
            r.ip = Set(Some("198.51.100.4".to_owned()));
            r.started_at = Set(Some(rows::at(19)));
            r.authorized_at = Set(Some(rows::at(21)));
            r.ended_at = Set(Some(rows::at(60)));
            r.note = Set(Some("controlled side".to_owned()));
            r.controller_note = Set(Some("controller side".to_owned()));
        })
        .await;

        let reply = list(&h, &admin, "connections", "").await;

        let full = &reply["items"][0];
        assert_eq!(full["device_id"], "100");
        assert_eq!(full["conn_id"], 2);
        assert_eq!(full["session_id"], "1002");
        assert_eq!(full["controller"], json!({ "id": bob, "username": "bob" }));
        assert_eq!(full["peer_id"], "900");
        assert_eq!(full["peer_name"], "<img src=x onerror=alert(1)>");
        assert_eq!(
            (full["conn_type"].as_i64(), full["primary_auth"].as_i64()),
            (Some(4), Some(2))
        );
        assert_eq!(full["two_factor"], 1);
        assert_eq!(full["started_at"], secs(19));
        assert_eq!(full["authorized_at"], secs(21));
        assert_eq!(full["ended_at"], secs(60));
        assert_eq!(full["created_at"], secs(20));
        assert_eq!(full["note"], "controlled side");
        assert_eq!(full["controller_note"], "controller side");
        assert!(full["guid"].as_str().is_some());
        let retired = &reply["items"][1];
        assert!(retired["session_id"].is_null(), "{retired}");
        assert!(retired["controller"].is_null());
        assert!(!reply.to_string().contains(RETIRED_PREFIX));
    })
    .await;
}

#[tokio::test]
async fn a_deleted_controller_leaves_a_row_without_controller() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        rows::connection(&h, "100", 1, 10, |r| r.controller_user_id = Set(Some(bob))).await;
        h.state.users.delete(bob).await.unwrap();

        let reply = list(&h, &admin, "connections", "").await;

        assert!(reply["items"][0]["controller"].is_null(), "{reply}");
    })
    .await;
}

#[tokio::test]
async fn listing_writes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        rows::connection(&h, "100", 1, 10, |_| {}).await;
        let before = h.audit_rows().await.len();

        for kind in KINDS {
            list(&h, &admin, kind, "").await;
        }

        assert_eq!(h.audit_rows().await.len(), before);
    })
    .await;
}

#[tokio::test]
async fn bad_queries_are_validation_errors() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let queries = [
            "sort=id",
            "sort=-device",
            "page=0",
            "page_size=201",
            "from=-1",
            "from=yesterday",
            "conn_type=x",
        ];
        for query in queries {
            let reply = h.get(&format!("{AUDIT}/connections?{query}"), &admin).await;
            assert_eq!(
                (reply.status.as_u16(), reply.code()),
                (422, "validation_failed"),
                "{query}"
            );
        }
        let long = "x".repeat(201);
        let reply = h
            .get(&format!("{AUDIT}/connections?q={long}"), &admin)
            .await;
        assert_eq!(reply.status, 422);
    })
    .await;
}

/// Control characters (a NUL makes PostgreSQL fail the query with a 500) are refused in every
/// text filter of every list.
#[tokio::test]
async fn control_characters_in_text_filters_are_validation_errors() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let filters = [
            ("connections", ["device", "q"].as_slice()),
            ("files", ["device", "q"].as_slice()),
            ("alarms", ["device"].as_slice()),
            ("logins", ["result", "device", "q"].as_slice()),
            ("console", ["action", "target_type", "q"].as_slice()),
        ];
        for (kind, params) in filters {
            for param in params {
                for bad in ["a%00b", "a%0Ab", "a%09b", "%1Fb", "a%7F"] {
                    let reply = h
                        .get(&format!("{AUDIT}/{kind}?{param}={bad}"), &admin)
                        .await;
                    assert_eq!(
                        (reply.status.as_u16(), reply.code()),
                        (422, "validation_failed"),
                        "{kind}?{param}={bad}"
                    );
                }
            }
        }
    })
    .await;
}

#[tokio::test]
async fn text_search_treats_like_wildcards_and_backslashes_literally() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let named = |name: &'static str| {
            move |r: &mut deskhaven_db::entity::connection_session::ActiveModel| {
                r.peer_name = Set(Some(name.to_owned()));
            }
        };
        let underscore = rows::connection(&h, "100", 1, 10, named("a_b")).await;
        rows::connection(&h, "100", 2, 20, named("axb")).await;
        let backslash = rows::connection(&h, "100", 3, 30, named(r"dir\file")).await;
        rows::connection(&h, "100", 4, 40, named("dirfile")).await;

        let one = list(&h, &admin, "connections", "q=a_b").await;
        let other = list(&h, &admin, "connections", "q=dir%5Cfile").await;
        let lone = list(&h, &admin, "connections", "q=%5C").await;

        assert_eq!(ids(&one), [underscore]);
        assert_eq!(ids(&other), [backslash]);
        assert_eq!(ids(&lone), [backslash]);
    })
    .await;
}
