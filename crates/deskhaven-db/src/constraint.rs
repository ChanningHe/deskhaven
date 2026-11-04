//! Backend-independent classification of constraint violations.

use sea_orm::{DbErr, RuntimeErr, SqlErr};

/// PostgreSQL `foreign_key_violation`.
const PG_FOREIGN_KEY_VIOLATION: &str = "23503";
/// PostgreSQL `restrict_violation`: PostgreSQL 18 reports `ON DELETE RESTRICT` with this code.
const PG_RESTRICT_VIOLATION: &str = "23001";
/// SQLite `SQLITE_CONSTRAINT_FOREIGNKEY` (787) and the extended code used for
/// `ON DELETE RESTRICT` (1811), which sea-orm does not classify.
const SQLITE_FOREIGN_KEY_CODES: [&str; 2] = ["787", "1811"];

/// Whether `err` is a foreign-key violation on either backend, including
/// `ON DELETE RESTRICT` (PostgreSQL 18 `23001`, SQLite extended code 1811).
pub fn is_foreign_key_violation(err: &DbErr) -> bool {
    if matches!(
        err.sql_err(),
        Some(SqlErr::ForeignKeyConstraintViolation(_))
    ) {
        return true;
    }
    database_code(err).is_some_and(|code| {
        code == PG_FOREIGN_KEY_VIOLATION
            || code == PG_RESTRICT_VIOLATION
            || SQLITE_FOREIGN_KEY_CODES.contains(&code.as_str())
    })
}

/// The driver-level error code, if the error came from the database itself.
pub(crate) fn database_code(err: &DbErr) -> Option<String> {
    let runtime = match err {
        DbErr::Exec(runtime) | DbErr::Query(runtime) | DbErr::Conn(runtime) => runtime,
        _ => return None,
    };
    match runtime {
        RuntimeErr::SqlxError(sqlx_err) => sqlx_err
            .as_database_error()
            .and_then(|db_err| db_err.code())
            .map(|code| code.into_owned()),
        _ => None,
    }
}
