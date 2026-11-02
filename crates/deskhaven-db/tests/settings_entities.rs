//! `settings` / `settings_history` entities against the migrated schema.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

use deskhaven_db::entity::{setting, setting_history};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::{ActiveModelTrait, ActiveValue::NotSet, ActiveValue::Set, EntityTrait};

fn ts(ms: i64) -> TimestampMs {
    time::from_millis(ms).unwrap()
}

fn setting_row(key: &str, version: i64) -> setting::ActiveModel {
    setting::ActiveModel {
        key: Set(key.to_owned()),
        value: Set(r#"{"enabled":true}"#.to_owned()),
        version: Set(version),
        updated_by: Set(None),
        updated_at: Set(ts(1_700_000_000_123)),
    }
}

fn history_row(key: &str, old: Option<&str>, new: &str) -> setting_history::ActiveModel {
    setting_history::ActiveModel {
        id: NotSet,
        key: Set(key.to_owned()),
        old_value: Set(old.map(str::to_owned)),
        new_value: Set(new.to_owned()),
        version: Set(1),
        updated_by: Set(Some(7)),
        updated_at: Set(time::now()),
        reason: Set(None),
    }
}

#[tokio::test]
async fn setting_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        let inserted = setting_row("auth.session_ttl", 1)
            .insert(&db.conn)
            .await
            .unwrap();

        let found = setting::Entity::find_by_id("auth.session_ttl")
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found, inserted, "{}", db.backend);
        assert_eq!(found.updated_at.timestamp_millis(), 1_700_000_000_123);
        assert_eq!(found.updated_by, None);
    })
    .await;
}

#[tokio::test]
async fn setting_key_is_primary_key() {
    each_migrated_backend(async |db: &TestDb| {
        setting_row("dup.key", 1).insert(&db.conn).await.unwrap();

        let res = setting_row("dup.key", 2).insert(&db.conn).await;

        assert!(res.is_err(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn setting_value_is_not_null() {
    each_migrated_backend(async |db: &TestDb| {
        let mut row = setting_row("no.value", 1);
        row.value = NotSet;

        assert!(row.insert(&db.conn).await.is_err(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn history_ids_are_generated_and_increasing() {
    each_migrated_backend(async |db: &TestDb| {
        let first = history_row("k", None, "1").insert(&db.conn).await.unwrap();
        let second = history_row("k", Some("1"), "2")
            .insert(&db.conn)
            .await
            .unwrap();

        assert!(second.id > first.id, "{}", db.backend);
        assert_eq!(first.old_value, None);
        assert_eq!(second.old_value.as_deref(), Some("1"));
        assert_eq!(second.updated_by, Some(7));

        let all = setting_history::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(all, vec![first, second], "{}", db.backend);
    })
    .await;
}
