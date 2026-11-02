//! Error rendering for logs and terminal output (06-security: no secrets in logs).
//!
//! Policy, applied to every startup, shutdown and `config check` error:
//! - Errors of deskhaven crates name keys, paths and causes, never values (key
//!   material, pin values, URLs); they are rendered with their source chain.
//! - Database errors (`DbErr`, `sqlx::Error`) can embed row contents
//!   (constraint details, decode failures) or connection parameters. The chain
//!   stops at the first one, which is reduced to its kind, the SQLSTATE /
//!   SQLite result code and a fixed description of well-known codes. Their own
//!   text is never logged, at any level.
//! - Database URLs are never logged, only the backend name
//!   ([`crate::database::backend`]).

use std::error::Error;

use sea_orm::sqlx;
use sea_orm::sqlx::error::{DatabaseError, ErrorKind};
use sea_orm::{ConnAcquireErr, DbErr, RuntimeErr};

/// Mask of the primary result code in an extended SQLite result code.
const SQLITE_PRIMARY_MASK: i32 = 0xff;

/// PostgreSQL SQLSTATEs an operator is likely to meet at startup.
const PG_CODES: &[(&str, &str)] = &[
    ("28P01", "password authentication failed"),
    (
        "28000",
        "connection rejected (check the role and pg_hba.conf)",
    ),
    ("3D000", "database does not exist"),
    ("42501", "insufficient privilege"),
    ("42P01", "table does not exist"),
    ("25006", "read-only transaction"),
    ("53300", "too many connections"),
    ("57P03", "the server is starting up or shutting down"),
];

/// SQLite primary result codes (`code & 0xff`).
const SQLITE_CODES: &[(i32, &str)] = &[
    (5, "database is locked"),
    (8, "attempt to write a read-only database"),
    (10, "disk I/O error"),
    (11, "database disk image is malformed"),
    (13, "database or disk is full"),
    (14, "unable to open database file"),
    (23, "authorization denied"),
    (26, "file is not a database"),
];

/// `context: cause: ...` with database errors reduced as described above.
pub fn render(err: &anyhow::Error) -> String {
    render_chain(err.chain())
}

/// [`render`] for a plain error and its sources.
pub fn render_error(err: &(dyn Error + 'static)) -> String {
    render_chain(std::iter::successors(Some(err), |e| (*e).source()))
}

fn render_chain<'a>(chain: impl Iterator<Item = &'a (dyn Error + 'static)>) -> String {
    let mut parts = Vec::new();
    for err in chain {
        if let Some(db) = err.downcast_ref::<DbErr>() {
            parts.push(describe_db_err(db));
            break;
        }
        if let Some(driver) = err.downcast_ref::<sqlx::Error>() {
            parts.push(describe_sqlx(driver));
            break;
        }
        parts.push(err.to_string());
    }
    parts.join(": ")
}

fn describe_db_err(err: &DbErr) -> String {
    match err {
        DbErr::ConnectionAcquire(ConnAcquireErr::Timeout) => {
            "timed out acquiring a database connection".to_owned()
        }
        DbErr::ConnectionAcquire(_) => "database connection closed".to_owned(),
        DbErr::Conn(rt) => format!("connection error: {}", describe_runtime(rt)),
        DbErr::Exec(rt) => format!("statement failed: {}", describe_runtime(rt)),
        DbErr::Query(rt) => format!("query failed: {}", describe_runtime(rt)),
        DbErr::RecordNotFound(_) => "record not found".to_owned(),
        DbErr::Migration(_) => "migration error".to_owned(),
        DbErr::TryIntoErr { .. } | DbErr::Type(_) | DbErr::Json(_) => {
            "value conversion error".to_owned()
        }
        _ => "database error".to_owned(),
    }
}

fn describe_runtime(err: &RuntimeErr) -> String {
    match err {
        RuntimeErr::SqlxError(driver) => describe_sqlx(driver),
        _ => "database driver error".to_owned(),
    }
}

fn describe_sqlx(err: &sqlx::Error) -> String {
    match err {
        sqlx::Error::Database(db) => describe_database_error(db.as_ref()),
        sqlx::Error::Io(io) => format!("I/O error ({})", io.kind()),
        // rustls / certificate failures; they carry no credentials or row data.
        sqlx::Error::Tls(tls) => format!("TLS error: {tls}"),
        sqlx::Error::Configuration(_) => "invalid database URL or connection options".to_owned(),
        sqlx::Error::PoolTimedOut => "timed out waiting for a database connection".to_owned(),
        sqlx::Error::PoolClosed => "the connection pool is closed".to_owned(),
        sqlx::Error::Protocol(_) => "database protocol error".to_owned(),
        _ => "database driver error".to_owned(),
    }
}

fn describe_database_error(err: &dyn DatabaseError) -> String {
    let code = err.code();
    let label = code
        .as_deref()
        .and_then(known_code)
        .unwrap_or_else(|| kind_label(err.kind()));
    match code {
        Some(code) => format!("database error {code}: {label}"),
        None => format!("database error: {label}"),
    }
}

/// SQLite codes are numeric (extended result codes), PostgreSQL's are SQLSTATEs.
fn known_code(code: &str) -> Option<&'static str> {
    match code.parse::<i32>() {
        Ok(number) => {
            let primary = number & SQLITE_PRIMARY_MASK;
            SQLITE_CODES
                .iter()
                .find(|(known, _)| *known == primary)
                .map(|(_, label)| *label)
        }
        Err(_) => PG_CODES
            .iter()
            .find(|(known, _)| *known == code)
            .map(|(_, label)| *label),
    }
}

fn kind_label(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::UniqueViolation => "unique constraint violated",
        ErrorKind::ForeignKeyViolation => "foreign key constraint violated",
        ErrorKind::NotNullViolation => "not-null constraint violated",
        ErrorKind::CheckViolation => "check constraint violated",
        _ => "rejected by the database",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Context;
    use deskhaven_db::{DbOptions, connect};
    use sea_orm::ConnectionTrait;

    use super::*;

    fn runtime(err: sqlx::Error) -> RuntimeErr {
        RuntimeErr::SqlxError(Arc::new(err))
    }

    #[test]
    fn io_errors_keep_only_their_kind() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "secret-detail");
        let err = DbErr::Conn(runtime(sqlx::Error::Io(io)));

        let text = render_error(&err);

        assert_eq!(text, "connection error: I/O error (connection refused)");
    }

    #[test]
    fn driver_text_is_dropped() {
        let err = DbErr::Query(runtime(sqlx::Error::Protocol("row-secret".into())));
        let custom = DbErr::Custom("row-secret".into());
        let config = sqlx::Error::Configuration("postgres://u:pw@h".into());

        for text in [
            render_error(&err),
            render_error(&custom),
            render_error(&config),
        ] {
            assert!(
                !text.contains("row-secret") && !text.contains("pw@"),
                "{text}"
            );
        }
    }

    #[test]
    fn chain_keeps_context_and_stops_at_the_database_error() {
        let db = deskhaven_db::DbError::Connect(DbErr::ConnectionAcquire(ConnAcquireErr::Timeout));
        let err = anyhow::Error::new(db).context("startup");

        assert_eq!(
            render(&err),
            "startup: failed to connect to the database: timed out acquiring a database connection"
        );
    }

    #[test]
    fn known_codes_are_described() {
        assert_eq!(known_code("28P01"), Some("password authentication failed"));
        // SQLITE_CANTOPEN_ISDIR (14 | 2 << 8) maps to its primary code.
        assert_eq!(known_code("526"), Some("unable to open database file"));
        assert_eq!(known_code("XX000"), None);
    }

    #[tokio::test]
    async fn constraint_violation_shows_code_not_message() {
        let db = connect(&DbOptions::new("sqlite::memory:")).await.unwrap();
        db.execute_unprepared(
            "CREATE TABLE t (v TEXT UNIQUE); INSERT INTO t (v) VALUES ('row-secret');",
        )
        .await
        .unwrap();

        let err = db
            .execute_unprepared("INSERT INTO t (v) VALUES ('row-secret')")
            .await
            .context("insert")
            .unwrap_err();

        let text = render(&err);
        assert_eq!(
            text,
            "insert: statement failed: database error 2067: unique constraint violated"
        );
    }
}
