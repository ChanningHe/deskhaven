//! `auth_lockouts` schema (M2-06): table, cleanup index, CHECKs and entity round trip, on
//! every available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

use deskhaven_db::entity::auth_lockout;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use sea_orm_migration::prelude::*;

const T0_MS: i64 = 1_700_000_000_123;

fn row(key: &str, failures: i32, lock_level: i32) -> auth_lockout::ActiveModel {
    let t0 = time::from_millis(T0_MS).unwrap();
    auth_lockout::ActiveModel {
        key: Set(key.to_owned()),
        failures: Set(failures),
        window_started_at: Set(t0),
        locked_until: Set(Some(t0)),
        lock_level: Set(lock_level),
        updated_at: Set(t0),
    }
}

#[tokio::test]
async fn table_and_cleanup_index_exist() {
    each_migrated_backend(async |db: &TestDb| {
        let manager = SchemaManager::new(&db.conn);
        assert!(manager.has_table("auth_lockouts").await.unwrap());
        let has_index = manager
            .has_index("auth_lockouts", "idx_auth_lockouts_updated_at")
            .await
            .unwrap();
        assert!(has_index, "{}: missing cleanup index", db.backend);
    })
    .await;
}

#[tokio::test]
async fn entity_round_trips() {
    each_migrated_backend(async |db: &TestDb| {
        let inserted = row("user:alice", 3, 2).insert(&db.conn).await.unwrap();

        let found = auth_lockout::Entity::find_by_id("user:alice")
            .one(&db.conn)
            .await
            .unwrap();

        assert_eq!(found, Some(inserted), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn key_is_unique() {
    each_migrated_backend(async |db: &TestDb| {
        row("user:alice", 1, 0).insert(&db.conn).await.unwrap();

        let duplicate = row("user:alice", 2, 0).insert(&db.conn).await;

        assert!(duplicate.is_err(), "{}: duplicate key accepted", db.backend);
    })
    .await;
}

#[tokio::test]
async fn negative_counters_are_rejected() {
    each_migrated_backend(async |db: &TestDb| {
        let failures = row("user:a", -1, 0).insert(&db.conn).await;
        let level = row("user:b", 0, -1).insert(&db.conn).await;

        assert!(failures.is_err(), "{}: negative failures", db.backend);
        assert!(level.is_err(), "{}: negative lock_level", db.backend);
    })
    .await;
}
