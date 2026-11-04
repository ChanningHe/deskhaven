//! Audit views (M3-08), part two: file, alarm, login and console lists, their wire shape and
//! filters. Runs on SQLite and PostgreSQL. Part one (permissions, connections, time window,
//! validation) is `audit_views.rs`.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::audit_rows::{self as rows, AUDIT, Change, ids, list, secs, setup};
use common::tokens;
use deskhaven_core::users::Role;
use deskhaven_db::entity::console_audit;
use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::entity::file_transfer::Direction;
use deskhaven_db::entity::login_event::{Client, Method};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;
use serde_json::{Value, json};

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
