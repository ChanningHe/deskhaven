//! Device group endpoints: list with device counts, detail, create, update, delete (its
//! devices become ungrouped, its strategy assignment goes), config-managed groups
//! read-only, audit rows. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::devices::{path, register};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::strategies::{NewStrategy, Options, Target};
use deskhaven_core::users::Role;
use deskhaven_db::entity::device_group;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use serde_json::{Value, json};

const GROUPS: &str = "/admin/api/v1/device-groups";

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

async fn create(h: &Harness, console: &Console, name: &str) -> i64 {
    let body = json!({ "name": name, "note": "n" });
    let reply = h.write(Method::POST, GROUPS, console, body).await;
    assert_eq!(reply.status, 201, "{}", reply.body);
    reply.body["id"].as_i64().unwrap()
}

fn names(reply: &Value) -> Vec<&str> {
    let items = reply["items"].as_array().unwrap();
    items.iter().map(|g| g["name"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn device_groups_are_created_listed_updated_and_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let office = create(&h, &admin, "Office").await;
        create(&h, &admin, "Lab").await;
        create(&h, &admin, "Office-2").await;
        let device = register(&h, "100").await;
        let body = json!({ "device_group_id": office });
        h.write(Method::PATCH, &path(device), &admin, body).await;

        let listed = h.get(&format!("{GROUPS}?sort=-name"), &admin).await;
        assert_eq!(names(&listed.body), ["Office-2", "Office", "Lab"]);
        assert_eq!(listed.body["items"][1]["device_count"], 1);
        let found = h.get(&format!("{GROUPS}?q=OFFICE"), &admin).await;
        assert_eq!(found.body["total"], 2);
        let bad = h.get(&format!("{GROUPS}?sort=device_count"), &admin).await;
        assert_eq!(bad.status, 422);

        let group = format!("{GROUPS}/{office}");
        let detail = h.get(&group, &admin).await;
        assert_eq!(
            (
                detail.body["managed_by"].as_str(),
                detail.body["device_count"].as_u64()
            ),
            (Some("ui"), Some(1))
        );
        let renamed = h
            .write(Method::PATCH, &group, &admin, json!({ "name": "HQ" }))
            .await;
        assert_eq!(renamed.body["name"], "HQ");
        let taken = h
            .write(Method::PATCH, &group, &admin, json!({ "name": "Lab" }))
            .await;
        assert_eq!((taken.status.as_u16(), taken.code()), (409, "conflict"));
        let duplicate = h
            .write(Method::POST, GROUPS, &admin, json!({ "name": "Lab" }))
            .await;
        assert_eq!(duplicate.status, 409);
        let blank = h
            .write(Method::POST, GROUPS, &admin, json!({ "name": " " }))
            .await;
        assert_eq!(blank.status, 422);

        assert_eq!(h.act(Method::DELETE, &group, &admin).await.status, 204);
        assert_eq!(h.get(&group, &admin).await.status, 404);
        assert_eq!(h.act(Method::DELETE, &group, &admin).await.status, 404);
        let ungrouped = h.get(&path(device), &admin).await;
        assert_eq!(ungrouped.body["device"]["device_group"], Value::Null);

        let rows = h.audit_rows().await;
        let shape: Vec<_> = rows
            .iter()
            .filter(|r| r.target_type == "device_group")
            .map(|r| (r.action.as_str(), r.before.is_some(), r.after.is_some()))
            .collect();
        assert_eq!(
            shape,
            [
                ("device_group.create", false, true),
                ("device_group.create", false, true),
                ("device_group.create", false, true),
                ("device_group.update", true, true),
                ("device_group.delete", true, false),
            ]
        );
    })
    .await;
}

#[tokio::test]
async fn an_empty_patch_changes_and_records_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let id = create(&h, &admin, "Office").await;
        let group = format!("{GROUPS}/{id}");
        let before = h.get(&group, &admin).await.body;
        let audits = h.audit_rows().await.len();

        let reply = h.write(Method::PATCH, &group, &admin, json!({})).await;

        assert_eq!(reply.status, 200, "{}", reply.body);
        assert_eq!(reply.body["updated_at"], before["updated_at"]);
        assert_eq!(h.audit_rows().await.len(), audits);
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_drops_its_strategy_assignment() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let group = create(&h, &admin, "Office").await;
        let strategy = NewStrategy::new("s", Options::from([("a".to_owned(), "b".to_owned())]));
        let strategy = h.state.strategies.create(strategy).await.unwrap();
        let target = Target::DeviceGroup(group);
        h.state
            .strategies
            .assign(target, strategy.id)
            .await
            .unwrap();

        h.act(Method::DELETE, &format!("{GROUPS}/{group}"), &admin)
            .await;
        assert_eq!(h.state.strategies.assignment(target).await.unwrap(), None);
    })
    .await;
}

#[tokio::test]
async fn config_managed_groups_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let now = time::now();
        let row = device_group::ActiveModel {
            name: Set("fleet".to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Config),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let id = row.insert(&h.state.db).await.unwrap().id;
        let group = format!("{GROUPS}/{id}");

        assert_eq!(h.get(&group, &admin).await.body["managed_by"], "config");
        let patch = h
            .write(Method::PATCH, &group, &admin, json!({ "note": "x" }))
            .await;
        assert_eq!(
            (patch.status.as_u16(), patch.code()),
            (409, "managed_by_config")
        );
        let delete = h.act(Method::DELETE, &group, &admin).await;
        assert_eq!(
            (delete.status.as_u16(), delete.code()),
            (409, "managed_by_config")
        );
        assert_eq!(h.get(&group, &admin).await.status, 200);
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}
