//! Who may use the device and device group endpoints: admins and owners only. Plain users
//! get 403 and change nothing, no session gets 401, a write without the CSRF header 403.
//! Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::devices::{path, register};
use common::{Console, Harness, harness};
use deskhaven_admin_api::AdminRoutes;
use deskhaven_core::users::Role;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use serde_json::{Value, json};

struct Call {
    method: Method,
    path: String,
    body: Option<Value>,
}

fn calls(device: i64, group: i64) -> Vec<Call> {
    let call = |method, path: String, body| Call { method, path, body };
    let device_path = path(device);
    let group_path = format!("/admin/api/v1/device-groups/{group}");
    vec![
        call(Method::GET, "/admin/api/v1/devices".to_owned(), None),
        call(Method::GET, device_path.clone(), None),
        call(
            Method::PATCH,
            device_path.clone(),
            Some(json!({ "note": "x" })),
        ),
        call(
            Method::POST,
            format!("{device_path}/disconnect"),
            Some(json!({ "conn_id": 1 })),
        ),
        call(Method::DELETE, device_path, None),
        call(Method::GET, "/admin/api/v1/device-groups".to_owned(), None),
        call(
            Method::POST,
            "/admin/api/v1/device-groups".to_owned(),
            Some(json!({ "name": "new" })),
        ),
        call(Method::GET, group_path.clone(), None),
        call(
            Method::PATCH,
            group_path.clone(),
            Some(json!({ "note": "x" })),
        ),
        call(Method::DELETE, group_path, None),
    ]
}

async fn send(h: &Harness, call: &Call, who: Option<&Console>, csrf: bool) -> (u16, String) {
    let cookie = who.map(|c| c.cookie.as_str());
    // Without a session any non-empty CSRF value passes, so the 401 shows.
    let token = match who {
        Some(console) => csrf.then_some(console.csrf.as_str()),
        None => Some("any"),
    };
    let reply = h
        .send(
            call.method.clone(),
            &call.path,
            cookie,
            token,
            call.body.clone(),
        )
        .await;
    (reply.status.as_u16(), reply.code().to_owned())
}

#[tokio::test]
async fn plain_users_and_strangers_are_refused_everywhere() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let user = h.console("plain", Role::User).await;
        let admin = h.console("adm", Role::Admin).await;
        let owner = h.console("own", Role::Owner).await;
        let device = register(&h, "100").await;
        let group = h.state.devices.create_group("Office", "").await.unwrap();

        for call in calls(device, group) {
            let cell = format!("{} {}", call.method, call.path);
            assert_eq!(
                send(&h, &call, Some(&user), true).await,
                (403, "forbidden".into()),
                "{cell}"
            );
            assert_eq!(
                send(&h, &call, None, true).await,
                (401, "unauthorized".into()),
                "{cell}"
            );
            if call.method != Method::GET {
                let got = send(&h, &call, Some(&admin), false).await;
                assert_eq!(got, (403, "csrf_failed".into()), "{cell}");
            }
        }
        assert!(
            h.audit_rows().await.is_empty(),
            "refused calls leave no audit row"
        );
        assert_eq!(
            h.state.devices.summary(device).await.unwrap().record.note,
            None
        );

        for who in [&admin, &owner] {
            for call in calls(device, group).iter().take(2) {
                let (status, _) = send(&h, call, Some(who), true).await;
                assert_eq!(status, 200, "{} {}", call.method, call.path);
            }
        }
    })
    .await;
}
