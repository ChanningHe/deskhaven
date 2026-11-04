//! Strategy endpoints (M3-07): the option catalog, CRUD with option patches, validation,
//! secret options (never returned, masked in audit rows) and configuration-managed
//! strategies. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::{Method, StatusCode};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::strategy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde_json::{Value, json};

const STRATEGIES: &str = "/admin/api/v1/strategies";
const SECRET: &str = "hunter2-proxy";

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

fn one(id: i64) -> String {
    format!("{STRATEGIES}/{id}")
}

/// Every audit before / after snapshot, as text.
async fn audit_text(h: &Harness) -> String {
    let rows = h.audit_rows().await;
    let snapshots = rows
        .iter()
        .flat_map(|r| [r.before.clone(), r.after.clone()]);
    snapshots.flatten().collect::<Vec<_>>().join("\n")
}

#[tokio::test]
async fn the_catalog_lists_typed_grouped_options_from_the_client_source() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;

        let reply = h.get("/admin/api/v1/strategy-catalog", &admin).await;

        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.body["source"]["rustdesk"], "23e5bb4");
        let options = reply.body["options"].as_array().unwrap();
        let find = |key: &str| options.iter().find(|o| o["key"] == key).unwrap().clone();
        let approve = find("approve-mode");
        assert_eq!(approve["kind"], "choice");
        assert_eq!(approve["group"], "security");
        assert_eq!(approve["choices"][1]["value"], "click");
        assert_eq!(approve["label"]["zh"], "会话接受方式");
        let keyboard = find("enable-keyboard");
        assert_eq!(
            (keyboard["kind"].clone(), keyboard["default"].clone()),
            (json!("bool"), json!("Y"))
        );
        let port = find("direct-access-port");
        assert_eq!(
            (port["min"].clone(), port["max"].clone()),
            (json!(1), json!(65535))
        );
        assert_eq!(find("proxy-password")["secret"], true);
        assert_eq!(find("whitelist")["secret"], false);
    })
    .await;
}

#[tokio::test]
async fn strategies_are_created_patched_listed_and_deleted_with_audit() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let body = json!({
            "name": "kiosk",
            "note": "front desk",
            "options": { "enable-keyboard": "N", "custom-x": "1", "proxy-password": SECRET },
        });

        let created = h.write(Method::POST, STRATEGIES, &admin, body).await;

        assert_eq!(created.status, StatusCode::CREATED, "{}", created.body);
        let id = created.body["id"].as_i64().unwrap();
        assert_eq!(
            created.body["options"],
            json!({ "enable-keyboard": "N", "custom-x": "1" }),
            "a secret value is never returned"
        );
        assert_eq!(created.body["secret_options"], json!(["proxy-password"]));
        assert_eq!(created.body["managed_by"], "ui");
        assert_eq!(created.body["assignment_count"], 0);
        assert_eq!(h.get(&one(id), &admin).await.body, created.body);

        let patch = json!({ "options": { "enable-keyboard": "Y", "custom-x": null, "new": "" } });
        let patched = h.write(Method::PATCH, &one(id), &admin, patch).await;

        assert_eq!(patched.status, StatusCode::OK, "{}", patched.body);
        assert_eq!(
            patched.body["options"],
            json!({ "enable-keyboard": "Y", "new": "" })
        );
        assert_eq!(
            patched.body["secret_options"],
            json!(["proxy-password"]),
            "kept"
        );
        assert_eq!(patched.body["note"], "front desk");
        let stored = h.state.strategies.get(id).await.unwrap().options;
        assert_eq!(stored["proxy-password"], SECRET);

        let cleared = json!({ "options": { "proxy-password": "" } });
        let cleared = h.write(Method::PATCH, &one(id), &admin, cleared).await;
        assert_eq!(
            cleared.body["options"]["proxy-password"], "",
            "empty is not secret"
        );
        assert_eq!(cleared.body["secret_options"], json!([]));

        let list = h
            .get(&format!("{STRATEGIES}?q=KIO&sort=-name"), &admin)
            .await;
        assert_eq!(list.status, StatusCode::OK, "{}", list.body);
        assert_eq!(
            (
                list.body["total"].clone(),
                list.body["items"][0]["name"].clone()
            ),
            (json!(1), json!("kiosk"))
        );
        let none = h.get(&format!("{STRATEGIES}?q=zzz"), &admin).await;
        assert_eq!(none.body["total"], 0);

        let deleted = h.act(Method::DELETE, &one(id), &admin).await;
        assert_eq!(deleted.status, StatusCode::NO_CONTENT);
        assert_eq!(h.get(&one(id), &admin).await.status, StatusCode::NOT_FOUND);

        let actions: Vec<_> = h
            .audit_rows()
            .await
            .into_iter()
            .map(|r| (r.action, r.target_type, r.target_id))
            .collect();
        let sid = id.to_string();
        assert_eq!(
            actions,
            [
                (
                    "strategy.create".to_owned(),
                    "strategy".to_owned(),
                    sid.clone()
                ),
                (
                    "strategy.update".to_owned(),
                    "strategy".to_owned(),
                    sid.clone()
                ),
                (
                    "strategy.update".to_owned(),
                    "strategy".to_owned(),
                    sid.clone()
                ),
                ("strategy.delete".to_owned(), "strategy".to_owned(), sid),
            ]
        );
        let audit = audit_text(&h).await;
        assert!(!audit.contains(SECRET), "{audit}");
        let first: Value =
            serde_json::from_str(h.audit_rows().await[0].after.as_ref().unwrap()).unwrap();
        assert_eq!(first["options"]["enable-keyboard"], "N");
        assert_eq!(first["name"], "kiosk");
    })
    .await;
}

#[tokio::test]
async fn invalid_input_is_rejected_without_a_change() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let ok = json!({ "name": "a", "options": {} });
        let id = h.write(Method::POST, STRATEGIES, &admin, ok).await.body["id"]
            .as_i64()
            .unwrap();
        let long = "v".repeat(4097);
        let bad_creates = [
            json!({ "name": "", "options": {} }),
            json!({ "name": "b", "options": { "bad key": "1" } }),
            json!({ "name": "b", "options": { "a/b": "1" } }),
            json!({ "name": "b", "options": { "enable-keyboard": "yes" } }),
            json!({ "name": "b", "options": { "approve-mode": "tap" } }),
            json!({ "name": "b", "options": { "direct-access-port": "70000" } }),
            json!({ "name": "b", "options": { "x": long } }),
            json!({ "name": "b", "options": { "x": "line\nbreak" } }),
        ];
        for body in bad_creates {
            let reply = h
                .write(Method::POST, STRATEGIES, &admin, body.clone())
                .await;
            assert_eq!(
                (reply.status, reply.code()),
                (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
                "{body}"
            );
        }
        let bad_patch = json!({ "options": { "enable-keyboard": "maybe" } });
        let reply = h.write(Method::PATCH, &one(id), &admin, bad_patch).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);

        let taken = h
            .write(Method::POST, STRATEGIES, &admin, json!({ "name": "a" }))
            .await;
        assert_eq!(
            (taken.status, taken.code()),
            (StatusCode::CONFLICT, "conflict")
        );
        let empty = h.write(Method::PATCH, &one(id), &admin, json!({})).await;
        assert_eq!(empty.status, StatusCode::OK);
        assert_eq!(
            h.audit_rows().await.len(),
            1,
            "only the first create is recorded"
        );
        let missing = h
            .write(Method::PATCH, &one(id + 99), &admin, json!({ "note": "x" }))
            .await;
        assert_eq!(
            (missing.status, missing.code()),
            (StatusCode::NOT_FOUND, "not_found")
        );
    })
    .await;
}

#[tokio::test]
async fn strategies_from_the_configuration_file_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let now = time::now();
        let row = strategy::ActiveModel {
            name: Set("pinned".to_owned()),
            options: Set("{}".to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Config),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let id = row.insert(&h.state.db).await.unwrap().id;

        let shown = h.get(&one(id), &admin).await;
        let patched = h
            .write(Method::PATCH, &one(id), &admin, json!({ "note": "x" }))
            .await;
        let deleted = h.act(Method::DELETE, &one(id), &admin).await;

        assert_eq!(shown.body["managed_by"], "config");
        assert_eq!(
            (patched.status, patched.code()),
            (StatusCode::CONFLICT, "managed_by_config")
        );
        assert_eq!(
            (deleted.status, deleted.code()),
            (StatusCode::CONFLICT, "managed_by_config")
        );
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}
