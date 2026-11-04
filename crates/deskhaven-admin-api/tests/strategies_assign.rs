//! Strategy assignments and the effective-strategy preview (M3-07), and the acceptance flow:
//! create a strategy, assign it to a device group, preview the merged result, and the next
//! heartbeat of the real client API delivers it. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::devices::{heartbeat_http, path, register};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

const STRATEGIES: &str = "/admin/api/v1/strategies";
/// The heartbeat refuses a device's beats closer than 2 s.
const BEAT_GAP: Duration = Duration::from_millis(2_100);

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

async fn create(h: &Harness, admin: &Console, name: &str, options: Value) -> i64 {
    let body = json!({ "name": name, "options": options });
    let reply = h.write(Method::POST, STRATEGIES, admin, body).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.body["id"].as_i64().unwrap()
}

fn assignments(id: i64) -> String {
    format!("{STRATEGIES}/{id}/assignments")
}

fn preview(device: i64) -> String {
    format!("/admin/api/v1/strategy-preview?device_id={device}")
}

async fn assign(h: &Harness, admin: &Console, id: i64, target: Value) -> StatusCode {
    h.write(Method::POST, &assignments(id), admin, target)
        .await
        .status
}

#[tokio::test]
async fn assigned_to_a_device_group_it_is_previewed_and_delivered_by_the_heartbeat() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let options = json!({ "enable-file-transfer": "N", "approve-mode": "click", "proxy-password": "pw-1" });
        let office = create(&h, &admin, "office", options).await;
        let group = h.state.devices.create_group("Office", "").await.unwrap();
        let device = register(&h, "100").await;
        let join = json!({ "device_group_id": group });
        assert_eq!(h.write(Method::PATCH, &path(device), &admin, join).await.status, StatusCode::OK);

        let target = json!({ "target_type": "device_group", "target_id": group });
        assert_eq!(assign(&h, &admin, office, target).await, StatusCode::NO_CONTENT);

        let listed = h.get(&assignments(office), &admin).await;
        assert_eq!(
            listed.body["items"],
            json!([{ "target_type": "device_group", "target_id": group, "target_name": "Office" }])
        );
        let shown = h.get(&preview(device), &admin).await;
        assert_eq!(shown.status, StatusCode::OK, "{}", shown.body);
        let merged: Vec<_> = shown.body["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| (o["key"].as_str().unwrap().to_owned(), o["value"].clone(), o["redacted"].clone(), o["source"]["scope"].clone(), o["source"]["strategy_name"].clone()))
            .collect();
        let dg = (json!("device_group"), json!("office"));
        assert_eq!(
            merged,
            [
                ("approve-mode".to_owned(), json!("click"), json!(false), dg.0.clone(), dg.1.clone()),
                ("enable-file-transfer".to_owned(), json!("N"), json!(false), dg.0.clone(), dg.1.clone()),
                ("proxy-password".to_owned(), json!(""), json!(true), dg.0.clone(), dg.1.clone()),
            ]
        );

        let (status, beat) = heartbeat_http(&h, "100", &[], 0).await;
        assert_eq!(status, StatusCode::OK, "{beat}");
        assert_eq!(
            beat["strategy"]["config_options"],
            json!({ "enable-file-transfer": "N", "approve-mode": "click", "proxy-password": "pw-1" }),
            "the client gets the secret in clear: that is how it applies it"
        );
        assert_eq!(beat["modified_at"], shown.body["modified_at"]);

        let patch = json!({ "options": { "approve-mode": null, "enable-audio": "N" } });
        assert_eq!(h.write(Method::PATCH, &format!("{STRATEGIES}/{office}"), &admin, patch).await.status, StatusCode::OK);
        tokio::time::sleep(BEAT_GAP).await;
        let (_, changed) = heartbeat_http(&h, "100", &[], beat["modified_at"].as_i64().unwrap()).await;
        assert_eq!(
            changed["strategy"]["config_options"],
            json!({ "enable-file-transfer": "N", "enable-audio": "N", "proxy-password": "pw-1", "approve-mode": "" }),
            "{changed}"
        );

        let remove = format!("{}?target_type=device_group&target_id={group}", assignments(office));
        assert_eq!(h.act(Method::DELETE, &remove, &admin).await.status, StatusCode::NO_CONTENT);
        tokio::time::sleep(BEAT_GAP).await;
        let (_, left) = heartbeat_http(&h, "100", &[], changed["modified_at"].as_i64().unwrap()).await;
        // Every key the device may still hold is cleared; the delivered-key set is a
        // superset, so the `approve-mode` removed earlier is cleared once more.
        assert_eq!(
            left["strategy"]["config_options"],
            json!({ "enable-file-transfer": "", "enable-audio": "", "proxy-password": "", "approve-mode": "" }),
            "{left}"
        );
    })
    .await;
}

#[tokio::test]
async fn assignment_targets_are_validated_and_replacing_is_audited() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let (a, b) = (
            create(&h, &admin, "a", json!({})).await,
            create(&h, &admin, "b", json!({})).await,
        );
        let user = h.create_user("alice", Role::User).await;
        let device = register(&h, "100").await;
        let bad = [
            json!({ "target_type": "global", "target_id": 1 }),
            json!({ "target_type": "device_group" }),
            json!({ "target_type": "planet", "target_id": 1 }),
            json!({ "target_type": "user", "target_id": user + 99 }),
        ];
        for body in bad {
            let reply = h
                .write(Method::POST, &assignments(a), &admin, body.clone())
                .await;
            assert_eq!(
                (reply.status, reply.code()),
                (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed"),
                "{body}"
            );
        }
        let missing = h
            .write(
                Method::POST,
                &assignments(a + 99),
                &admin,
                json!({ "target_type": "global" }),
            )
            .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);

        for (id, target) in [
            (a, json!({ "target_type": "global" })),
            (a, json!({ "target_type": "user", "target_id": user })),
            (a, json!({ "target_type": "device", "target_id": device })),
            (b, json!({ "target_type": "user", "target_id": user })),
        ] {
            assert_eq!(assign(&h, &admin, id, target).await, StatusCode::NO_CONTENT);
        }

        let of_a = h.get(&assignments(a), &admin).await.body["items"].clone();
        assert_eq!(
            of_a,
            json!([
                { "target_type": "global", "target_id": null, "target_name": null },
                { "target_type": "device", "target_id": device, "target_name": "100" },
            ])
        );
        assert_eq!(
            h.get(&format!("{STRATEGIES}/{a}"), &admin).await.body["assignment_count"],
            2
        );
        let not_theirs = format!("{}?target_type=user&target_id={user}", assignments(a));
        let reply = h.act(Method::DELETE, &not_theirs, &admin).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::NOT_FOUND, "not_found")
        );
        let global = format!("{}?target_type=global", assignments(a));
        assert_eq!(
            h.act(Method::DELETE, &global, &admin).await.status,
            StatusCode::NO_CONTENT
        );

        let rows = h.audit_rows().await;
        let replaced = rows
            .iter()
            .filter(|r| r.action == "strategy.assign")
            .nth(3)
            .unwrap();
        let before: Value = serde_json::from_str(replaced.before.as_ref().unwrap()).unwrap();
        let after: Value = serde_json::from_str(replaced.after.as_ref().unwrap()).unwrap();
        assert_eq!(
            before,
            json!({ "strategy_id": a, "target_type": "user", "target_id": user })
        );
        assert_eq!(
            after,
            json!({ "strategy_id": b, "target_type": "user", "target_id": user })
        );
        assert_eq!(replaced.target_id, b.to_string());
        let unassigned = rows
            .iter()
            .filter(|r| r.action == "strategy.unassign")
            .count();
        assert_eq!(unassigned, 1, "the refused removal records nothing");
    })
    .await;
}

#[tokio::test]
async fn the_preview_of_an_unknown_device_is_not_found() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let reply = h.get(&preview(4242), &admin).await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::NOT_FOUND, "not_found")
        );
        let device = register(&h, "100").await;
        let empty = h.get(&preview(device), &admin).await;
        assert_eq!(empty.body["layers"], json!([]));
        assert_eq!(empty.body["options"], json!([]));
    })
    .await;
}
