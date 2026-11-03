//! Schema migrations (forward-only).
//!
//! Naming: `mYYYYMMDD_NNNNNN_<name>` (timestamp, not a sequence number), registered in order
//! in [`Migrator::migrations`]. One migration body serves both backends; dialect differences
//! branch on `manager.get_database_backend()`.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;

mod m20261102_000001_settings;
mod m20261102_000002_devices;
mod m20261102_000010_users_groups;
mod m20261102_000011_external_identity;
mod m20261102_000012_sessions_credentials;
mod m20261102_000013_password_changed_at;
mod m20261102_000014_auth_lockouts;
mod m20261102_000020_device_groups;
mod m20261102_000021_devices_foreign_keys;
mod m20261102_000022_address_books;
mod m20261102_000023_strategies;
mod m20261102_000024_audit;
mod m20261103_000025_address_book_peer_hash_enc;
mod m20261103_000026_settings_history_reset;
mod m20261103_000027_device_strategy_keys;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20261102_000001_settings::Migration),
            Box::new(m20261102_000002_devices::Migration),
            Box::new(m20261102_000010_users_groups::Migration),
            Box::new(m20261102_000011_external_identity::Migration),
            Box::new(m20261102_000012_sessions_credentials::Migration),
            Box::new(m20261102_000013_password_changed_at::Migration),
            Box::new(m20261102_000014_auth_lockouts::Migration),
            Box::new(m20261102_000020_device_groups::Migration),
            Box::new(m20261102_000021_devices_foreign_keys::Migration),
            Box::new(m20261102_000022_address_books::Migration),
            Box::new(m20261102_000023_strategies::Migration),
            Box::new(m20261102_000024_audit::Migration),
            Box::new(m20261103_000025_address_book_peer_hash_enc::Migration),
            Box::new(m20261103_000026_settings_history_reset::Migration),
            Box::new(m20261103_000027_device_strategy_keys::Migration),
        ]
    }
}

/// Named `CHECK (col IN (...))` for an enum stored as `TEXT` (04-data-model §1).
fn enum_check(name: &str, col: impl IntoColumnRef, values: &[&str]) -> Check {
    Check::named(
        name.to_owned(),
        Expr::col(col).is_in(values.iter().copied()),
    )
}

/// SQLite stores booleans as integers and accepts any value; constrain them to 0/1.
/// PostgreSQL's native `boolean` needs nothing.
fn bool_checks(table: &mut TableCreateStatement, backend: DbBackend, name: &str, cols: &[&str]) {
    if backend != DbBackend::Sqlite {
        return;
    }
    for col in cols {
        let expr = Expr::col(Alias::new(*col)).is_in([0, 1]);
        table.check(Check::named(format!("ck_{name}_{col}_bool"), expr));
    }
}

/// Named UNIQUE index over one column. NULLs never conflict on either backend, so a nullable
/// column is unique among its non-NULL values.
fn unique_index(name: &str, table: impl IntoIden, col: impl IntoIden) -> IndexCreateStatement {
    Index::create()
        .name(name)
        .table(table)
        .col(col)
        .unique()
        .to_owned()
}

/// Named non-unique index over one column (PostgreSQL does not index FK columns itself).
fn index(name: &str, table: impl IntoIden, col: impl IntoIden) -> IndexCreateStatement {
    Index::create().name(name).table(table).col(col).to_owned()
}

/// Inline foreign key (SQLite cannot add one to an existing table).
fn foreign_key(
    name: &str,
    from: (impl IntoIden, impl IntoIden),
    to: (impl IntoIden, impl IntoIden),
    on_delete: ForeignKeyAction,
) -> ForeignKeyCreateStatement {
    ForeignKey::create()
        .name(name)
        .from(from.0, from.1)
        .to(to.0, to.1)
        .on_delete(on_delete)
        .to_owned()
}
