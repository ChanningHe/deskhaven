//! `GET /overview` (M3-14): permissions and the correctness of every number, on SQLite and
//! PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::net::{IpAddr, Ipv4Addr};

use axum::http::Method;
use common::audit_rows::{T0, alarm, connection, login};
use common::devices::register;
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::entity::login_event::{Client, Method as LoginMethod};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ConnectionTrait;
use serde_json::Value;

const OVERVIEW: &str = "/admin/api/v1/overview";
const HOUR: i64 = 3600;

async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

/// Seconds after [`T0`] that are `ago` seconds before now, for the row builders.
fn secs_ago(ago: i64) -> i64 {
    (time::now().timestamp_millis() - T0) / 1000 - ago
}

async fn overview(h: &Harness, admin: &Console) -> Value {
    let reply = h.get(OVERVIEW, admin).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    reply.body
}

async fn set_version(h: &Harness, rustdesk_id: &str, version: &str) {
    let sql =
        format!("UPDATE devices SET version = '{version}' WHERE rustdesk_id = '{rustdesk_id}'");
    h.state.db.execute_unprepared(&sql).await.unwrap();
}

#[tokio::test]
async fn only_admins_read_the_overview() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let user = h.console("bob", Role::User).await;
        let owner = h.console("own", Role::Owner).await;
        let client_session = h.client_session_cookie("carol").await;

        let denied = h.get(OVERVIEW, &user).await;
        assert_eq!((denied.status.as_u16(), denied.code()), (403, "forbidden"));
        let anonymous = h.send(Method::GET, OVERVIEW, None, None, None).await;
        assert_eq!(anonymous.status, 401);
        let client = h
            .send(Method::GET, OVERVIEW, Some(&client_session), None, None)
            .await;
        assert_eq!(client.status, 401);
        assert_eq!(h.get(OVERVIEW, &admin).await.status, 200);
        assert_eq!(h.get(OVERVIEW, &owner).await.status, 200);
    })
    .await;
}

#[tokio::test]
async fn an_empty_deployment_reads_as_zeros() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let body = overview(&h, &admin).await;
        assert_eq!(body["devices"]["online"], 0);
        assert_eq!(body["devices"]["total"], 0);
        assert_eq!(body["active_connections"], 0);
        assert_eq!(body["connections_24h"], 0);
        assert_eq!(body["failed_logins_24h"], 0);
        assert_eq!(body["recent_alarms"], serde_json::json!([]));
        assert_eq!(body["client_versions"]["versions"], serde_json::json!([]));
        assert_eq!(body["client_versions"]["other"], 0);
        assert_eq!(body["client_versions"]["unknown"], 0);
        assert!(body["generated_at"].as_i64().unwrap() > T0);
    })
    .await;
}

#[tokio::test]
async fn devices_and_live_connections_come_from_the_registry_and_presence() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        for id in ["100", "200", "300", "400"] {
            register(&h, id).await;
        }
        let presence = h.state.devices.presence();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));
        presence.heartbeat("100", ip, &[1, 2]);
        presence.heartbeat("200", ip, &[3]);
        presence.signal_online("300", ip);
        presence.signal_online("400", ip);
        presence.signal_offline("400");

        let body = overview(&h, &admin).await;
        assert_eq!(body["devices"]["total"], 4);
        assert_eq!(body["devices"]["online"], 3);
        assert_eq!(body["active_connections"], 3);
    })
    .await;
}

#[tokio::test]
async fn connections_and_failed_logins_count_the_last_24_hours_only() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        for (conn, ago) in [(1, 60), (2, 23 * HOUR), (3, 25 * HOUR), (4, 400 * HOUR)] {
            connection(&h, "100", conn, secs_ago(ago), |_| {}).await;
        }
        let password = (LoginMethod::Password, Client::Console);
        let totp = (LoginMethod::Totp, Client::Rustdesk);
        let events = [
            ("success", password, 60),
            ("invalid_credentials", password, 120),
            ("locked", totp, 23 * HOUR),
            ("invalid_code", totp, 10 * HOUR),
            ("tfa_required", password, 60),
            ("tfa_setup_required", password, 60),
            ("invalid_credentials", password, 25 * HOUR),
        ];
        for (result, kind, ago) in events {
            login(&h, None, "mallory", kind, result, secs_ago(ago)).await;
        }

        let body = overview(&h, &admin).await;
        assert_eq!(body["connections_24h"], 2);
        assert_eq!(body["failed_logins_24h"], 3);
    })
    .await;
}

#[tokio::test]
async fn recent_alarms_are_the_latest_few_newest_first() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let mut ids = Vec::new();
        for typ in 0..8_i32 {
            let info = format!(r#"{{"name":"dev{typ}"}}"#);
            let ago = (8 - i64::from(typ)) * 100;
            ids.push(alarm(&h, "100", typ, &info, secs_ago(ago)).await);
        }

        let body = overview(&h, &admin).await;
        let alarms = body["recent_alarms"].as_array().unwrap();
        let got: Vec<i64> = alarms.iter().map(|a| a["id"].as_i64().unwrap()).collect();
        let want: Vec<i64> = ids.iter().rev().take(5).copied().collect();
        assert_eq!(got, want);
        assert_eq!(alarms[0]["device_id"], "100");
        assert_eq!(alarms[0]["typ"], 7);
        assert_eq!(alarms[0]["info"][0]["key"], "name");
        assert_eq!(alarms[0]["info"][0]["value"], "dev7");
    })
    .await;
}

#[tokio::test]
async fn client_versions_list_the_top_few_then_other_and_unknown() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        let versions = [
            ("1.4.1", 3),
            ("1.3.0", 2),
            ("1.2.0", 1),
            ("1.1.0", 1),
            ("1.0.0", 1),
            ("0.9.0", 1),
            ("0.8.0", 1),
        ];
        let mut n = 0;
        for (version, count) in versions {
            for _ in 0..count {
                n += 1;
                let id = (1000 + n).to_string();
                register(&h, &id).await;
                set_version(&h, &id, version).await;
            }
        }
        register(&h, "2000").await;
        register(&h, "2001").await;
        set_version(&h, "2001", "").await;

        let body = overview(&h, &admin).await;
        assert_eq!(body["devices"]["total"], 12);
        let versions = &body["client_versions"];
        let top: Vec<(&str, i64)> = versions["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| (v["version"].as_str().unwrap(), v["count"].as_i64().unwrap()))
            .collect();
        // Most devices first; ties in version order.
        assert_eq!(
            top,
            [
                ("1.4.1", 3),
                ("1.3.0", 2),
                ("0.8.0", 1),
                ("0.9.0", 1),
                ("1.0.0", 1)
            ]
        );
        assert_eq!(versions["other"], 2);
        assert_eq!(versions["unknown"], 2);
    })
    .await;
}
