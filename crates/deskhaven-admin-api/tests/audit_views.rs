//! Audit views (M3-08): connection / file / alarm / login / console audit lists with their
//! filters, order, pagination and wire shape, admin only. Runs on SQLite and PostgreSQL.
//! The 100k-row timing is in `audit_views_perf.rs`.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::audit_rows::{self as rows, Change, T0};
use common::{Console, Harness, harness, tokens};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::audit::RETIRED_PREFIX;
use deskhaven_core::users::Role;
use deskhaven_db::entity::console_audit;
use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::entity::file_transfer::Direction;
use deskhaven_db::entity::login_event::{Client, Method};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::EntityTrait;
use serde_json::{Value, json};

const AUDIT: &str = "/admin/api/v1/audit";
const KINDS: [&str; 5] = ["connections", "files", "alarms", "logins", "console"];

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

fn ids(reply: &Value) -> Vec<i64> {
    let items = reply["items"].as_array().unwrap();
    items.iter().map(|i| i["id"].as_i64().unwrap()).collect()
}

async fn list(h: &Harness, admin: &Console, kind: &str, query: &str) -> Value {
    let reply = h.get(&format!("{AUDIT}/{kind}?{query}"), admin).await;
    assert_eq!(reply.status, 200, "{kind}?{query}: {}", reply.body);
    reply.body
}

fn secs(n: i64) -> i64 {
    T0 + n * 1000
}

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
async fn files_filter_by_device_direction_and_path() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let a = rows::transfer(&h, "100", Direction::Send, "C:\\Users\\x\\a.txt", 10).await;
        let b = rows::transfer(&h, "100", Direction::Receive, "/tmp/Report.pdf", 20).await;
        let c = rows::transfer(&h, "200", Direction::Send, "/etc/passwd", 30).await;

        let all = list(&h, &admin, "files", "").await;
        assert_eq!(ids(&all), [c, b, a]);
        assert_eq!(ids(&list(&h, &admin, "files", "device=100").await), [b, a]);
        let sent = list(&h, &admin, "files", "direction=send").await;
        assert_eq!(ids(&sent), [c, a]);
        assert_eq!(ids(&list(&h, &admin, "files", "q=REPORT").await), [b]);
        let window = format!("from={}&to={}", secs(15), secs(25));
        assert_eq!(ids(&list(&h, &admin, "files", &window).await), [b]);

        let item = &all["items"][2];
        assert_eq!(item["device_id"], "100");
        assert_eq!(item["peer_id"], "900");
        assert_eq!(item["direction"], "send");
        assert_eq!(item["path"], "C:\\Users\\x\\a.txt");
        assert_eq!(
            (item["is_file"].as_bool(), item["file_count"].as_i64()),
            (Some(true), Some(2))
        );
        assert_eq!(
            item["files"],
            json!([{ "name": "a.txt", "size": 10 }, { "name": "b.bin", "size": 2048 }])
        );
        assert_eq!(item["ip"], "198.51.100.4");

        let bad = h
            .get(&format!("{AUDIT}/files?direction=sideways"), &admin)
            .await;
        assert_eq!(
            (bad.status.as_u16(), bad.code()),
            (422, "validation_failed")
        );
    })
    .await;
}

#[tokio::test]
async fn a_malformed_file_list_degrades_to_an_empty_list() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = rows::transfer(&h, "100", Direction::Send, "/x", 10).await;
        sea_orm::ConnectionTrait::execute_unprepared(
            &h.state.db,
            &format!("UPDATE file_transfers SET files = 'not json' WHERE id = {id}"),
        )
        .await
        .unwrap();

        let reply = list(&h, &admin, "files", "").await;

        assert_eq!(reply["items"][0]["files"], json!([]));
    })
    .await;
}

#[tokio::test]
async fn alarms_filter_by_device_and_type_and_flatten_info() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let info = r#"{"ip":"198.51.100.4","name":"<b>x</b>","tries":31,"ok":false}"#;
        let a = rows::alarm(&h, "100", 1, info, 10).await;
        let b = rows::alarm(&h, "100", 0, "{}", 20).await;
        let c = rows::alarm(&h, "200", 1, "[1,2]", 30).await;

        let all = list(&h, &admin, "alarms", "").await;
        assert_eq!(ids(&all), [c, b, a]);
        assert_eq!(ids(&list(&h, &admin, "alarms", "typ=1").await), [c, a]);
        assert_eq!(ids(&list(&h, &admin, "alarms", "device=100").await), [b, a]);

        let item = &all["items"][2];
        assert_eq!(
            (item["typ"].as_i64(), item["conn_id"].as_i64()),
            (Some(1), Some(7))
        );
        assert_eq!(item["device_id"], "100");
        let entries = item["info"].as_array().unwrap();
        let get = |key: &str| {
            let hit = entries.iter().find(|e| e["key"] == key).unwrap();
            hit["value"].as_str().unwrap().to_owned()
        };
        assert_eq!(get("ip"), "198.51.100.4");
        assert_eq!(get("name"), "<b>x</b>");
        assert_eq!(
            (get("tries"), get("ok")),
            ("31".to_owned(), "false".to_owned())
        );
        assert_eq!(all["items"][1]["info"], json!([]));
        assert_eq!(all["items"][0]["info"], json!([]));
    })
    .await;
}

#[tokio::test]
async fn logins_filter_by_user_method_result_client_and_text() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let bob = h.create_user("bob", Role::User).await;
        let pw = (Method::Password, Client::Console);
        let a = rows::login(&h, Some(bob), "bob", pw, "success", 10).await;
        let b = rows::login(&h, None, "mallory", pw, "invalid_credentials", 20).await;
        let c = rows::login(
            &h,
            Some(bob),
            "bob",
            (Method::Totp, Client::Rustdesk),
            "invalid_code",
            30,
        )
        .await;

        let all = list(&h, &admin, "logins", "").await;
        assert_eq!(ids(&all), [c, b, a]);
        assert_eq!(
            ids(&list(&h, &admin, "logins", &format!("user={bob}")).await),
            [c, a]
        );
        assert_eq!(ids(&list(&h, &admin, "logins", "method=totp").await), [c]);
        assert_eq!(
            ids(&list(&h, &admin, "logins", "client=console").await),
            [b, a]
        );
        let failed = list(&h, &admin, "logins", "result=invalid_credentials").await;
        assert_eq!(ids(&failed), [b]);
        assert_eq!(ids(&list(&h, &admin, "logins", "q=MALL").await), [b]);

        assert_eq!(
            all["items"][2]["user"],
            json!({ "id": bob, "username": "bob" })
        );
        assert!(all["items"][1]["user"].is_null());
        assert_eq!(all["items"][1]["username_attempted"], "mallory");
        assert_eq!(all["items"][0]["method"], "totp");
        assert_eq!(all["items"][0]["client"], "rustdesk");
        assert_eq!(all["items"][0]["result"], "invalid_code");
        assert_eq!(all["items"][0]["ip"], "203.0.113.9");

        let bad = h.get(&format!("{AUDIT}/logins?method=magic"), &admin).await;
        assert_eq!(
            (bad.status.as_u16(), bad.code()),
            (422, "validation_failed")
        );
    })
    .await;
}

#[tokio::test]
async fn console_audit_names_actors_and_shows_snapshots_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let created = tokens::create(&h, &admin, "ci", &["deploy"]).await;
        let token_id = tokens::id_of(&created);
        // Creating the token audited itself; the test wants only its own rows.
        let cleared = console_audit::Entity::delete_many().exec(&h.state.db).await;
        assert_eq!(cleared.unwrap().rows_affected, 1);
        let change = |actor, action, target| Change {
            actor,
            action,
            target,
            before: None,
            after: None,
        };
        let user = (ActorKind::User, Some(admin.user_id));
        let a = rows::change(
            &h,
            Change {
                before: Some(r#"{"role":"user"}"#),
                after: Some(r#"{"role":"admin","password":"<redacted>"}"#),
                ..change(user, "user.update", ("user", "5"))
            },
            10,
        )
        .await;
        let b = rows::change(
            &h,
            change(
                (ActorKind::ApiToken, Some(token_id)),
                "device.assign",
                ("device", "100"),
            ),
            20,
        )
        .await;
        let purge = r#"{"kind":"conn","deleted":12,"cutoff":1,"batch":500}"#;
        let c = rows::change(
            &h,
            Change {
                after: Some(purge),
                ..change(
                    (ActorKind::System, None),
                    "audit.retention.purge",
                    ("audit", "0198f7a2-0000-7000-8000-000000000001"),
                )
            },
            30,
        )
        .await;

        let all = list(&h, &admin, "console", "").await;
        assert_eq!(ids(&all), [c, b, a]);
        let by_user = list(&h, &admin, "console", &format!("user={}", admin.user_id)).await;
        assert_eq!(ids(&by_user), [a]);
        assert_eq!(
            ids(&list(&h, &admin, "console", "actor_kind=system").await),
            [c]
        );
        assert_eq!(
            ids(&list(&h, &admin, "console", "action=device.assign").await),
            [b]
        );
        assert_eq!(
            ids(&list(&h, &admin, "console", "target_type=audit").await),
            [c]
        );
        assert_eq!(ids(&list(&h, &admin, "console", "q=retention").await), [c]);

        let first = &all["items"][2];
        assert_eq!(
            first["actor"],
            json!({ "kind": "user", "id": admin.user_id, "name": "adm" })
        );
        assert_eq!(first["before"], json!({ "role": "user" }));
        assert_eq!(first["after"]["password"], "<redacted>");
        assert_eq!(
            (first["target_type"].as_str(), first["target_id"].as_str()),
            (Some("user"), Some("5"))
        );
        assert_eq!(all["items"][1]["actor"]["name"], "ci");
        let system = &all["items"][0];
        assert_eq!(
            system["actor"],
            json!({ "kind": "system", "id": null, "name": null })
        );
        assert_eq!(
            system["after"],
            serde_json::from_str::<Value>(purge).unwrap()
        );
        assert!(system["before"].is_null());
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
