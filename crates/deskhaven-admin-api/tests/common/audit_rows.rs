//! Row builders of the audit view tests: one audit record per call, written straight to the
//! tables (the ingest paths have their own tests in core).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test crate uses a subset.
#![allow(dead_code)]

use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::entity::file_transfer::Direction;
use deskhaven_db::entity::login_event::{Client, Method};
use deskhaven_db::entity::{alarm, connection_session, console_audit, file_transfer, login_event};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection};

use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::testing::TestDb;
use serde_json::Value;

use super::{Console, Harness, harness};

/// A fixed epoch for the rows' times, so tests read in seconds after it.
pub const T0: i64 = 1_700_000_000_000;

/// `T0` plus `secs` seconds, as a row timestamp.
pub fn at(secs: i64) -> TimestampMs {
    time::from_millis(T0 + secs * 1000).unwrap()
}

fn db(h: &Harness) -> &DatabaseConnection {
    &h.state.db
}

/// A connection of `device` created `secs` after [`T0`]; `edit` sets the optional columns.
/// Returns the row id.
pub async fn connection(
    h: &Harness,
    device: &str,
    conn_id: i64,
    secs: i64,
    edit: impl FnOnce(&mut connection_session::ActiveModel),
) -> i64 {
    let mut row = connection_session::ActiveModel {
        guid: Set(format!("guid-{device}-{conn_id}-{secs}")),
        device_rustdesk_id: Set(device.to_owned()),
        conn_id: Set(conn_id),
        session_id: Set(format!("{}", 1000 + conn_id)),
        created_at: Set(at(secs)),
        ..Default::default()
    };
    edit(&mut row);
    row.insert(db(h)).await.unwrap().id
}

pub async fn transfer(
    h: &Harness,
    device: &str,
    direction: Direction,
    path: &str,
    secs: i64,
) -> i64 {
    file_transfer::ActiveModel {
        device_rustdesk_id: Set(device.to_owned()),
        peer_rustdesk_id: Set("900".to_owned()),
        conn_id: Set(1),
        direction: Set(direction),
        path: Set(path.to_owned()),
        is_file: Set(true),
        file_count: Set(2),
        files: Set(r#"[["a.txt",10],["b.bin",2048]]"#.to_owned()),
        ip: Set(Some("198.51.100.4".to_owned())),
        created_at: Set(at(secs)),
        ..Default::default()
    }
    .insert(db(h))
    .await
    .unwrap()
    .id
}

pub async fn alarm(h: &Harness, device: &str, typ: i32, info: &str, secs: i64) -> i64 {
    alarm::ActiveModel {
        device_rustdesk_id: Set(device.to_owned()),
        typ: Set(typ),
        info: Set(info.to_owned()),
        conn_id: Set(7),
        created_at: Set(at(secs)),
        ..Default::default()
    }
    .insert(db(h))
    .await
    .unwrap()
    .id
}

pub async fn login(
    h: &Harness,
    user_id: Option<i64>,
    username: &str,
    (method, client): (Method, Client),
    result: &str,
    secs: i64,
) -> i64 {
    login_event::ActiveModel {
        user_id: Set(user_id),
        username_attempted: Set(username.to_owned()),
        method: Set(method),
        result: Set(result.to_owned()),
        client: Set(client),
        rustdesk_id: Set(None),
        ip: Set(Some("203.0.113.9".to_owned())),
        created_at: Set(at(secs)),
        ..Default::default()
    }
    .insert(db(h))
    .await
    .unwrap()
    .id
}

/// One console audit row.
pub struct Change<'a> {
    pub actor: (ActorKind, Option<i64>),
    pub action: &'a str,
    pub target: (&'a str, &'a str),
    pub before: Option<&'a str>,
    pub after: Option<&'a str>,
}

pub async fn change(h: &Harness, c: Change<'_>, secs: i64) -> i64 {
    console_audit::ActiveModel {
        actor_kind: Set(c.actor.0),
        actor_id: Set(c.actor.1),
        action: Set(c.action.to_owned()),
        target_type: Set(c.target.0.to_owned()),
        target_id: Set(c.target.1.to_owned()),
        before: Set(c.before.map(str::to_owned)),
        after: Set(c.after.map(str::to_owned)),
        ip: Set(None),
        created_at: Set(at(secs)),
        ..Default::default()
    }
    .insert(db(h))
    .await
    .unwrap()
    .id
}
pub const AUDIT: &str = "/admin/api/v1/audit";
pub const KINDS: [&str; 5] = ["connections", "files", "alarms", "logins", "console"];

pub async fn setup(db: &TestDb) -> (Harness, Console) {
    let h = harness(db, AdminRoutes::new()).await;
    let admin = h.console("adm", Role::Admin).await;
    (h, admin)
}

pub fn ids(reply: &Value) -> Vec<i64> {
    let items = reply["items"].as_array().unwrap();
    items.iter().map(|i| i["id"].as_i64().unwrap()).collect()
}

pub async fn list(h: &Harness, admin: &Console, kind: &str, query: &str) -> Value {
    let reply = h.get(&format!("{AUDIT}/{kind}?{query}"), admin).await;
    assert_eq!(reply.status, 200, "{kind}?{query}: {}", reply.body);
    reply.body
}

pub fn secs(n: i64) -> i64 {
    T0 + n * 1000
}
