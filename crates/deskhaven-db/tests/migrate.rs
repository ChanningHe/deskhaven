//! Migrator acceptance tests, run on every available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

use deskhaven_db::migration::Migrator;
use deskhaven_db::testing::{TestDb, each_backend};
use deskhaven_db::{DbError, migrate, migrate_with};
use sea_orm::{ConnectionTrait, DatabaseConnection, EntityTrait};
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::big_integer;
use sea_orm_migration::seaql_migrations;

const FAILING_VERSION: &str = "m20991231_000001_always_fails";
const PARTIAL_TABLE: &str = "partial_from_failed_migration";

/// Test-only migration: applies some DDL, then fails.
struct AlwaysFails;

impl MigrationName for AlwaysFails {
    fn name(&self) -> &str {
        FAILING_VERSION
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AlwaysFails {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = Table::create()
            .table(Alias::new(PARTIAL_TABLE))
            .col(big_integer("id"))
            .to_owned();
        manager.create_table(table).await?;
        Err(DbErr::Migration("intentional failure".to_owned()))
    }
}

/// The real migrations followed by one that always fails.
struct BrokenMigrator;

impl MigratorTrait for BrokenMigrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        let mut all = Migrator::migrations();
        all.push(Box::new(AlwaysFails));
        all
    }
}

async fn applied_versions(conn: &DatabaseConnection) -> Vec<String> {
    let manager = SchemaManager::new(conn);
    if !manager.has_table("seaql_migrations").await.unwrap() {
        return Vec::new();
    }
    seaql_migrations::Entity::find()
        .all(conn)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.version)
        .collect()
}

fn expected_versions() -> Vec<String> {
    Migrator::migrations()
        .iter()
        .map(|m| m.name().to_owned())
        .collect()
}

async fn has_table(conn: &DatabaseConnection, table: &str) -> bool {
    SchemaManager::new(conn).has_table(table).await.unwrap()
}

#[tokio::test]
async fn migrate_creates_settings_tables_and_records_versions() {
    each_backend(async |db: &TestDb| {
        migrate(&db.conn).await.unwrap();

        assert!(has_table(&db.conn, "settings").await, "{}", db.backend);
        assert!(
            has_table(&db.conn, "settings_history").await,
            "{}",
            db.backend
        );
        assert_eq!(applied_versions(&db.conn).await, expected_versions());
    })
    .await;
}

#[tokio::test]
async fn migrate_is_idempotent() {
    each_backend(async |db: &TestDb| {
        migrate(&db.conn).await.unwrap();
        migrate(&db.conn).await.unwrap();

        assert_eq!(applied_versions(&db.conn).await, expected_versions());
    })
    .await;
}

#[tokio::test]
async fn failed_migration_on_fresh_db_rolls_back_everything() {
    each_backend(async |db: &TestDb| {
        let err = migrate_with::<BrokenMigrator>(&db.conn).await.unwrap_err();

        assert!(matches!(err, DbError::Migration(_)), "{err:?}");
        assert!(
            applied_versions(&db.conn).await.is_empty(),
            "{}",
            db.backend
        );
        assert!(!has_table(&db.conn, PARTIAL_TABLE).await, "{}", db.backend);
        assert!(!has_table(&db.conn, "settings").await, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn failed_migration_leaves_no_version_record_on_migrated_db() {
    each_backend(async |db: &TestDb| {
        migrate(&db.conn).await.unwrap();

        let err = migrate_with::<BrokenMigrator>(&db.conn).await.unwrap_err();

        assert!(matches!(err, DbError::Migration(_)), "{err:?}");
        let versions = applied_versions(&db.conn).await;
        assert!(
            !versions.iter().any(|v| v == FAILING_VERSION),
            "{}",
            db.backend
        );
        assert_eq!(versions, expected_versions());
        assert!(!has_table(&db.conn, PARTIAL_TABLE).await, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn migrate_refuses_db_with_unknown_applied_version() {
    each_backend(async |db: &TestDb| {
        migrate(&db.conn).await.unwrap();
        let insert = Query::insert()
            .into_table(seaql_migrations::Entity)
            .columns([
                seaql_migrations::Column::Version,
                seaql_migrations::Column::AppliedAt,
            ])
            .values_panic(["m20991231_000002_from_newer_binary".into(), 0i64.into()])
            .to_owned();
        db.conn.execute(&insert).await.unwrap();

        let err = migrate(&db.conn).await.unwrap_err();

        assert!(matches!(err, DbError::Migration(_)), "{err:?}");
    })
    .await;
}

#[test]
fn migration_names_are_timestamped_and_ordered() {
    let names = expected_versions();
    assert!(!names.is_empty());
    for name in &names {
        assert!(is_timestamped_name(name), "bad migration name: {name}");
    }
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        names, sorted,
        "migrations must be unique and in timestamp order"
    );
}

/// `mYYYYMMDD_NNNNNN_<snake_case>`
fn is_timestamped_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let all_digits = |r: std::ops::Range<usize>| bytes[r].iter().all(u8::is_ascii_digit);
    bytes.len() > 17
        && bytes[0] == b'm'
        && all_digits(1..9)
        && bytes[9] == b'_'
        && all_digits(10..16)
        && bytes[16] == b'_'
        && bytes[17..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}
