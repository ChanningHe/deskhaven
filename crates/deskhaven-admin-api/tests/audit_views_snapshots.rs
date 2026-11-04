//! Audit views (M3-08): console snapshots are redacted again when read, and text that does
//! not parse is never shown. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::audit_rows::{self as rows, Change, ids, list, setup};
use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::testing::{TestDb, each_migrated_backend};

#[tokio::test]
async fn snapshots_are_redacted_again_on_read() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        // A row as an older writer, or a hand-edited database, could have left it.
        let leaky = r#"{"name":"ci","api_key":"sk-live-123","nested":{"password":"hunter2"}}"#;
        let id = rows::change(
            &h,
            Change {
                actor: (ActorKind::User, Some(admin.user_id)),
                action: "setting.update",
                target: ("setting", "x"),
                before: None,
                after: Some(leaky),
            },
            10,
        )
        .await;

        let reply = list(&h, &admin, "console", "").await;

        assert_eq!(ids(&reply), [id]);
        let shown = reply.to_string();
        assert!(!shown.contains("sk-live-123"), "{shown}");
        assert!(!shown.contains("hunter2"), "{shown}");
        let after = &reply["items"][0]["after"];
        assert_eq!(after["name"], "ci");
        assert_eq!(after["api_key"], "<redacted>");
        assert_eq!(after["nested"]["password"], "<redacted>");
    })
    .await;
}

#[tokio::test]
async fn a_snapshot_that_is_not_json_is_replaced_by_a_placeholder() {
    each_migrated_backend(async |db: &TestDb| {
        let (h, admin) = setup(db).await;
        rows::change(
            &h,
            Change {
                actor: (ActorKind::Cli, None),
                action: "setting.update",
                target: ("setting", "y"),
                before: Some("not json, password=hunter2"),
                after: None,
            },
            10,
        )
        .await;

        let reply = list(&h, &admin, "console", "").await;

        assert!(!reply.to_string().contains("hunter2"), "{reply}");
        assert_eq!(reply["items"][0]["before"], "<unreadable>");
        assert!(reply["items"][0]["after"].is_null());
    })
    .await;
}
