//! Settings endpoints: who may call them (owner only, gate and in-transaction re-check)
//! and what the console audit rows hold (before / after values, secrets as `{"set": bool}`).
//! Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::entity::user;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

const SETTINGS: &str = "/admin/api/v1/settings";
const SECRET: &str = "smtp-pass-0042";

fn patch_body(key: &str, value: Value) -> Value {
    json!({ "changes": [{ "key": key, "version": 0, "value": value }] })
}

async fn call(h: &Harness, who: &Console, call: &str) -> (u16, String) {
    let history = format!("{SETTINGS}/smtp.port/history");
    let reply = match call {
        "list" => h.get(SETTINGS, who).await,
        "history" => h.get(&history, who).await,
        _ => {
            h.write(
                Method::PATCH,
                SETTINGS,
                who,
                patch_body("smtp.port", json!(2525)),
            )
            .await
        }
    };
    (reply.status.as_u16(), reply.code().to_owned())
}

#[tokio::test]
async fn only_owners_pass_for_every_settings_endpoint() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let user = h.console("usr", Role::User).await;
        let admin = h.console("adm", Role::Admin).await;
        let owner = h.console("own", Role::Owner).await;

        for endpoint in ["list", "history", "update"] {
            for (who, name) in [(&user, "user"), (&admin, "admin")] {
                let got = call(&h, who, endpoint).await;
                assert_eq!(got, (403, "forbidden".to_owned()), "{name} {endpoint}");
            }
            let (status, _) = call(&h, &owner, endpoint).await;
            assert_eq!(status, 200, "owner {endpoint}");
        }
        // Denied writes changed nothing; only the owner's own write is audited.
        let list = h.get(SETTINGS, &owner).await;
        let port = list
            .body
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["key"] == "smtp.port")
            .unwrap();
        assert_eq!(port["value"], 2525);
        assert_eq!(h.audit_rows().await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn anonymous_and_csrf_less_requests_are_refused() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let owner = h.console("own", Role::Owner).await;
        let body = Some(patch_body("smtp.port", json!(2525)));

        let anonymous = h.send(Method::GET, SETTINGS, None, None, None).await;
        assert_eq!(anonymous.status, 401);
        let no_session = h
            .send(Method::PATCH, SETTINGS, None, Some("x"), body.clone())
            .await;
        assert_eq!(no_session.status, 401);
        let no_csrf = h
            .send(
                Method::PATCH,
                SETTINGS,
                Some(&owner.cookie),
                None,
                body.clone(),
            )
            .await;
        assert_eq!(
            (no_csrf.status.as_u16(), no_csrf.code()),
            (403, "csrf_failed")
        );
        let wrong_csrf = h
            .send(
                Method::PATCH,
                SETTINGS,
                Some(&owner.cookie),
                Some("forged"),
                body,
            )
            .await;
        assert_eq!(wrong_csrf.status, 403);
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn a_write_rechecks_that_the_actor_is_still_the_owner() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        h.create_user("keeper", Role::Owner).await;
        let owner = h.console("own", Role::Owner).await;
        assert_eq!(call(&h, &owner, "list").await.0, 200);
        // Demoted behind the services' back: the session cache still says owner.
        user::Entity::update_many()
            .col_expr(user::Column::Role, Expr::value(Role::Admin))
            .filter(user::Column::Id.eq(owner.user_id))
            .exec(&h.state.db)
            .await
            .unwrap();

        assert_eq!(
            call(&h, &owner, "list").await.0,
            200,
            "the gate still trusts the session"
        );
        let write = call(&h, &owner, "update").await;

        assert_eq!(write, (403, "forbidden".to_owned()));
        assert_eq!(
            h.state
                .settings
                .snapshot()
                .view("smtp.port")
                .unwrap()
                .version,
            0
        );
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}

#[tokio::test]
async fn audit_rows_hold_before_and_after_values_and_never_a_secret() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, owner) = {
            let h = harness(db, AdminRoutes::new()).await;
            let owner = h.console("own", Role::Owner).await;
            (h, owner)
        };

        let update = h
            .write(
                Method::PATCH,
                SETTINGS,
                &owner,
                json!({ "changes": [
                    { "key": "auth.password.min_length", "version": 0, "value": 12 },
                    { "key": "smtp.password", "version": 0, "value": SECRET },
                    { "key": "smtp.port", "version": 0, "value": 2525 },
                ], "reason": "tuning the mail relay" }),
            )
            .await;
        assert_eq!(update.status, 200, "{}", update.body);
        let reset = h
            .write(
                Method::PATCH,
                SETTINGS,
                &owner,
                json!({ "changes": [{ "key": "smtp.password", "version": 1, "reset": true }] }),
            )
            .await;
        assert_eq!(reset.status, 200, "{}", reset.body);

        let rows = h.audit_rows().await;
        let shape: Vec<_> = rows
            .iter()
            .map(|r| (r.action.as_str(), r.target_type.as_str(), r.actor_id))
            .collect();
        assert_eq!(
            shape,
            [
                ("setting.update", "setting", Some(owner.user_id)),
                ("setting.reset", "setting", Some(owner.user_id)),
            ]
        );
        assert_eq!(
            rows[0].target_id,
            "auth.password.min_length,smtp.password,smtp.port"
        );
        let before: Value = serde_json::from_str(rows[0].before.as_deref().unwrap()).unwrap();
        let after: Value = serde_json::from_str(rows[0].after.as_deref().unwrap()).unwrap();
        // Defaults before; the password key name does not blank the number behind it.
        assert_eq!(
            before,
            json!({ "changes": [
                { "key": "auth.password.min_length", "value": 10 },
                { "key": "smtp.password", "value": { "set": false } },
                { "key": "smtp.port", "value": 587 },
            ] })
        );
        // The note travels with the change; a batch without one has no `reason`.
        assert_eq!(
            after,
            json!({
                "changes": [
                    { "key": "auth.password.min_length", "value": 12 },
                    { "key": "smtp.password", "value": { "set": true } },
                    { "key": "smtp.port", "value": 2525 },
                ],
                "reason": "tuning the mail relay",
            })
        );
        let reset_after: Value = serde_json::from_str(rows[1].after.as_deref().unwrap()).unwrap();
        assert_eq!(
            reset_after,
            json!({ "changes": [{ "key": "smtp.password", "value": { "set": false }, "reset": true }] })
        );
        let everything = format!("{rows:?}");
        assert!(!everything.contains(SECRET));
        assert!(rows.iter().all(|r| r.ip.is_some()));
    })
    .await;
}

#[tokio::test]
async fn rejected_batches_leave_no_audit_row() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let owner = h.console("own", Role::Owner).await;
        let bad = h
            .write(
                Method::PATCH,
                SETTINGS,
                &owner,
                patch_body("smtp.port", json!(0)),
            )
            .await;
        let stale = h
            .write(
                Method::PATCH,
                SETTINGS,
                &owner,
                json!({ "changes": [{ "key": "smtp.port", "version": 5, "value": 25 }] }),
            )
            .await;
        assert_eq!((bad.status.as_u16(), stale.status.as_u16()), (422, 409));
        assert!(h.audit_rows().await.is_empty());
    })
    .await;
}
