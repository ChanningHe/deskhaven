//! M4-05: `users.email_norm` is the ASCII lowercase of `email`. Earlier rows were folded with
//! Unicode `to_lowercase`, which maps e.g. the Kelvin sign (U+212A) to ASCII `k`; the
//! migration recomputes them. Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used)]

use deskhaven_db::begin_write;
use deskhaven_db::migration::Migrator;
use deskhaven_db::testing::{TestDb, each_backend};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;

/// Applies the next `steps` pending migrations (all when `None`).
async fn migrate_steps(conn: &DatabaseConnection, steps: Option<u32>) {
    let txn = begin_write(conn).await.unwrap();
    Migrator::up(&txn, steps).await.unwrap();
    txn.commit().await.unwrap();
}

async fn insert_user(conn: &DatabaseConnection, username: &str, email: &str, norm: &str) {
    let sql = "INSERT INTO users (username, username_norm, display_name, email, email_norm, \
               auth_source, role, status, created_at, updated_at) \
               VALUES ($1, $1, $1, $2, $3, 'local', 'user', 'active', 0, 0)";
    let backend = conn.get_database_backend();
    let sql = match backend {
        sea_orm::DbBackend::Sqlite => sql.replace('$', "?"),
        _ => sql.to_owned(),
    };
    let stmt =
        Statement::from_sql_and_values(backend, sql, [username.into(), email.into(), norm.into()]);
    conn.execute_raw(stmt).await.unwrap();
}

async fn norm_of(conn: &DatabaseConnection, username: &str) -> String {
    let backend = conn.get_database_backend();
    let sql = match backend {
        sea_orm::DbBackend::Sqlite => "SELECT email_norm FROM users WHERE username = ?",
        _ => "SELECT email_norm FROM users WHERE username = $1",
    };
    let stmt = Statement::from_sql_and_values(backend, sql, [username.into()]);
    let row = conn.query_one_raw(stmt).await.unwrap().unwrap();
    row.try_get("", "email_norm").unwrap()
}

#[tokio::test]
async fn the_migration_recomputes_email_norm_as_ascii_lowercase() {
    each_backend(async |db: &TestDb| {
        let before = Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20261106_000029_email_norm_ascii")
            .unwrap();
        migrate_steps(&db.conn, Some(u32::try_from(before).unwrap())).await;
        insert_user(
            &db.conn,
            "kelvin",
            "\u{212A}elvin@Corp.example",
            "kelvin@corp.example",
        )
        .await;
        insert_user(&db.conn, "uni", "Ünï@Corp.example", "ünï@corp.example").await;
        insert_user(&db.conn, "bob", "Bob@Corp.example", "bob@corp.example").await;

        migrate_steps(&db.conn, None).await;

        assert_eq!(
            norm_of(&db.conn, "kelvin").await,
            "\u{212A}elvin@corp.example"
        );
        assert_eq!(norm_of(&db.conn, "uni").await, "Ünï@corp.example");
        assert_eq!(norm_of(&db.conn, "bob").await, "bob@corp.example");
        // The ASCII address the Kelvin sign used to collide with is free now.
        insert_user(
            &db.conn,
            "kevin",
            "kelvin@corp.example",
            "kelvin@corp.example",
        )
        .await;
    })
    .await;
}
