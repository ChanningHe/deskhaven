//! Device endpoints: list (filters, sort, presence), detail (sysinfo, effective strategy
//! and its sources, recent connections), update (owner, device group, note, status) and
//! delete, with their audit rows. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::devices::{heartbeat, path, register, report};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::strategies::{NewStrategy, Options, Target};
use deskhaven_core::users::Role;
use deskhaven_db::entity::connection_session;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use serde_json::{Value, json};

const DEVICES: &str = "/admin/api/v1/devices";

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

fn rustdesk_ids(reply: &Value) -> Vec<&str> {
    let items = reply["items"].as_array().unwrap();
    items
        .iter()
        .map(|d| d["rustdesk_id"].as_str().unwrap())
        .collect()
}

fn options(pairs: &[(&str, &str)]) -> Options {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

#[tokio::test]
async fn list_filters_sorts_and_reports_presence() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        for id in ["100", "200", "300"] {
            register(&h, id).await;
        }
        report(&h, "100", "alpha", "linux / Ubuntu").await;
        report(&h, "200", "beta", "windows / 11").await;
        heartbeat(&h, "200", &[]);

        let all = h.get(&format!("{DEVICES}?sort=-rustdesk_id"), &admin).await;
        assert_eq!(all.status, 200, "{}", all.body);
        assert_eq!(rustdesk_ids(&all.body), ["300", "200", "100"]);
        assert_eq!(all.body["total"], 3);
        let online: Vec<_> = all.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["online"].as_bool().unwrap())
            .collect();
        assert_eq!(online, [false, true, false]);
        assert_eq!(all.body["items"][1]["hostname"], "beta");
        assert!(all.body["items"][1]["last_seen_at"].as_i64().unwrap() > 0);

        let live = h.get(&format!("{DEVICES}?online=true"), &admin).await;
        assert_eq!(
            (rustdesk_ids(&live.body), live.body["total"].as_u64()),
            (vec!["200"], Some(1))
        );
        let offline = h
            .get(&format!("{DEVICES}?online=false&sort=rustdesk_id"), &admin)
            .await;
        assert_eq!(rustdesk_ids(&offline.body), ["100", "300"]);
        let paged = h
            .get(
                &format!("{DEVICES}?online=false&sort=rustdesk_id&page=2&page_size=1"),
                &admin,
            )
            .await;
        assert_eq!(
            (rustdesk_ids(&paged.body), paged.body["total"].as_u64()),
            (vec!["300"], Some(2))
        );

        let by_name = h.get(&format!("{DEVICES}?q=ALP"), &admin).await;
        assert_eq!(rustdesk_ids(&by_name.body), ["100"]);
        let by_os = h.get(&format!("{DEVICES}?os=windows"), &admin).await;
        assert_eq!(rustdesk_ids(&by_os.body), ["200"]);
        let none = h.get(&format!("{DEVICES}?q=%25"), &admin).await;
        assert_eq!(none.body["total"], 0);

        let bad = h.get(&format!("{DEVICES}?sort=pk"), &admin).await;
        assert_eq!(
            (bad.status.as_u16(), bad.code()),
            (422, "validation_failed")
        );
    })
    .await;
}

#[tokio::test]
async fn list_filters_by_owner_group_and_status() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let (first, second) = (register(&h, "100").await, register(&h, "200").await);
        register(&h, "300").await;
        let bob = h.create_user("bob", Role::User).await;
        let office = h.state.devices.create_group("Office", "").await.unwrap();
        let body = json!({ "owner_user_id": bob, "device_group_id": office });
        let moved = h.write(Method::PATCH, &path(first), &admin, body).await;
        assert_eq!(moved.status, 200, "{}", moved.body);
        let disabled = json!({ "status": "disabled" });
        h.write(Method::PATCH, &path(second), &admin, disabled)
            .await;

        let owned = h.get(&format!("{DEVICES}?owner={bob}"), &admin).await;
        assert_eq!(rustdesk_ids(&owned.body), ["100"]);
        assert_eq!(owned.body["items"][0]["owner"]["username"], "bob");
        assert_eq!(owned.body["items"][0]["device_group"]["name"], "Office");
        let grouped = h
            .get(&format!("{DEVICES}?device_group={office}"), &admin)
            .await;
        assert_eq!(rustdesk_ids(&grouped.body), ["100"]);
        let off = h.get(&format!("{DEVICES}?status=disabled"), &admin).await;
        assert_eq!(rustdesk_ids(&off.body), ["200"]);
    })
    .await;
}

#[tokio::test]
async fn detail_shows_sysinfo_strategy_with_sources_and_recent_connections() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        report(&h, "100", "alpha", "linux / Ubuntu").await;
        heartbeat(&h, "100", &[7, 9]);
        let office = h.state.devices.create_group("Office", "").await.unwrap();
        let base = options(&[("theme", "dark"), ("lang", "en")]);
        let base = h
            .state
            .strategies
            .create(NewStrategy::new("base", base))
            .await;
        let night = options(&[("theme", "light")]);
        let night = h
            .state
            .strategies
            .create(NewStrategy::new("night", night))
            .await;
        let (base, night) = (base.unwrap(), night.unwrap());
        h.state
            .strategies
            .assign(Target::Global, base.id)
            .await
            .unwrap();
        let target = Target::DeviceGroup(office);
        h.state.strategies.assign(target, night.id).await.unwrap();
        let body = json!({ "device_group_id": office, "note": "front desk" });
        h.write(Method::PATCH, &path(id), &admin, body).await;
        connection(&h, "100", 7, Some(time::now()), None).await;
        connection(&h, "100", 3, Some(time::now()), Some(time::now())).await;
        connection(&h, "999", 1, Some(time::now()), None).await;

        let reply = h.get(&path(id), &admin).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        let device = &reply.body["device"];
        assert_eq!(device["rustdesk_id"], "100");
        assert_eq!(device["online"], true);
        assert_eq!(device["note"], "front desk");
        assert_eq!(device["cpu"], "Intel, 3.2GHz, 8/4 cores");
        assert_eq!(device["memory"], "15.5GB");
        assert_eq!(reply.body["connections"], json!([7, 9]));
        assert!(reply.body.get("pk").is_none() && device.get("uuid").is_none());

        let strategy = &reply.body["strategy"];
        let layers: Vec<_> = strategy["layers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    l["scope"].as_str().unwrap(),
                    l["strategy_name"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(layers, [("global", "base"), ("device_group", "night")]);
        let sources: Vec<_> = strategy["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                let value = o["value"].as_str().unwrap();
                (
                    o["key"].as_str().unwrap(),
                    value,
                    o["source"]["scope"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            sources,
            [("lang", "en", "global"), ("theme", "light", "device_group")]
        );

        let recent = reply.body["recent_connections"].as_array().unwrap();
        let conn_ids: Vec<_> = recent
            .iter()
            .map(|c| c["conn_id"].as_i64().unwrap())
            .collect();
        assert_eq!(conn_ids.len(), 2);
        assert!(conn_ids.contains(&7) && conn_ids.contains(&3));
        assert_eq!(h.get(&path(id + 1000), &admin).await.status, 404);
        assert_eq!(h.get(&format!("{DEVICES}/nope"), &admin).await.status, 404);
    })
    .await;
}

async fn connection(
    h: &Harness,
    device: &str,
    conn_id: i64,
    started: Option<time::TimestampMs>,
    ended: Option<time::TimestampMs>,
) {
    let row = connection_session::ActiveModel {
        guid: Set(format!("guid-{device}-{conn_id}")),
        device_rustdesk_id: Set(device.to_owned()),
        conn_id: Set(conn_id),
        session_id: Set("s".to_owned()),
        peer_rustdesk_id: Set(Some("555".to_owned())),
        started_at: Set(started),
        ended_at: Set(ended),
        created_at: Set(time::now()),
        ..Default::default()
    };
    row.insert(&h.state.db).await.unwrap();
}

#[tokio::test]
async fn update_assigns_owner_group_note_and_status_with_audit() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        let bob = h.create_user("bob", Role::User).await;
        let office = h.state.devices.create_group("Office", "").await.unwrap();

        let body = json!({ "owner_user_id": bob, "device_group_id": office, "note": "n1" });
        let updated = h.write(Method::PATCH, &path(id), &admin, body).await;
        assert_eq!(updated.status, 200, "{}", updated.body);
        assert_eq!(updated.body["owner"]["id"], bob);
        assert_eq!(updated.body["device_group"]["id"], office);
        assert_eq!(updated.body["note"], "n1");

        let disabled = h
            .write(
                Method::PATCH,
                &path(id),
                &admin,
                json!({ "status": "disabled" }),
            )
            .await;
        assert_eq!(disabled.body["status"], "disabled");
        assert_eq!(disabled.body["owner"]["id"], bob, "absent fields stay");

        let cleared = json!({ "owner_user_id": null, "device_group_id": null, "note": "" });
        let cleared = h.write(Method::PATCH, &path(id), &admin, cleared).await;
        assert_eq!(cleared.body["owner"], Value::Null);
        assert_eq!(cleared.body["device_group"], Value::Null);
        assert_eq!(cleared.body["note"], "");

        let shape: Vec<_> = h
            .audit_rows()
            .await
            .iter()
            .map(|r| (r.action.clone(), r.target_type.clone(), r.target_id.clone()))
            .collect();
        let target = |action: &str| (action.to_owned(), "device".to_owned(), id.to_string());
        assert_eq!(
            shape,
            [
                target("device.update"),
                target("device.disable"),
                target("device.update")
            ]
        );
        let rows = h.audit_rows().await;
        let after: Value = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
        assert_eq!(after["owner_user_id"], bob);
        let before: Value = serde_json::from_str(rows[0].before.as_deref().unwrap()).unwrap();
        assert_eq!(before["owner_user_id"], Value::Null);
    })
    .await;
}

#[tokio::test]
async fn update_rejects_unknown_references_and_devices_and_keeps_the_row() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        let bad_owner = json!({ "owner_user_id": 4242 });
        let owner = h.write(Method::PATCH, &path(id), &admin, bad_owner).await;
        assert_eq!(
            (owner.status.as_u16(), owner.code()),
            (422, "validation_failed")
        );
        let bad_group = json!({ "device_group_id": 4242, "note": "kept out" });
        let group = h.write(Method::PATCH, &path(id), &admin, bad_group).await;
        assert_eq!(group.status, 422);
        let long = json!({ "note": "x".repeat(2001) });
        assert_eq!(
            h.write(Method::PATCH, &path(id), &admin, long).await.status,
            422
        );
        let unknown = json!({ "uuid": "x" });
        assert_eq!(
            h.write(Method::PATCH, &path(id), &admin, unknown)
                .await
                .status,
            422
        );
        let missing = h
            .write(
                Method::PATCH,
                &path(id + 1000),
                &admin,
                json!({ "note": "x" }),
            )
            .await;
        assert_eq!(
            (missing.status.as_u16(), missing.code()),
            (404, "not_found")
        );

        let device = h.get(&path(id), &admin).await;
        assert_eq!(device.body["device"]["note"], "");
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn moving_a_device_changes_its_effective_strategy_at_once() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        let office = h.state.devices.create_group("Office", "").await.unwrap();
        let strategy = NewStrategy::new("office", options(&[("theme", "dark")]));
        let strategy = h.state.strategies.create(strategy).await.unwrap();
        h.state
            .strategies
            .assign(Target::DeviceGroup(office), strategy.id)
            .await
            .unwrap();
        let before = h.get(&path(id), &admin).await;
        assert_eq!(before.body["strategy"]["options"], json!([]));

        let body = json!({ "device_group_id": office });
        h.write(Method::PATCH, &path(id), &admin, body).await;
        let after = h.get(&path(id), &admin).await;
        assert_eq!(after.body["strategy"]["options"][0]["key"], "theme");
    })
    .await;
}

#[tokio::test]
async fn delete_removes_the_device_and_its_assignment_and_is_audited() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = register(&h, "100").await;
        heartbeat(&h, "100", &[]);
        let strategy = NewStrategy::new("s", options(&[("a", "b")]));
        let strategy = h.state.strategies.create(strategy).await.unwrap();
        h.state
            .strategies
            .assign(Target::Device(id), strategy.id)
            .await
            .unwrap();

        let deleted = h.act(Method::DELETE, &path(id), &admin).await;
        assert_eq!(deleted.status, 204);
        assert_eq!(h.get(&path(id), &admin).await.status, 404);
        assert_eq!(h.act(Method::DELETE, &path(id), &admin).await.status, 404);
        assert!(!h.state.devices.presence().is_online("100"));
        let assignment = h
            .state
            .strategies
            .assignment(Target::Device(id))
            .await
            .unwrap();
        assert_eq!(assignment, None);

        let rows = h.audit_rows().await;
        assert_eq!(rows.len(), 1, "the failed repeat leaves no row");
        let row = &rows[0];
        assert_eq!(
            (row.action.as_str(), row.target_id.as_str()),
            ("device.delete", id.to_string().as_str())
        );
        assert!(row.before.is_some() && row.after.is_none());
    })
    .await;
}
