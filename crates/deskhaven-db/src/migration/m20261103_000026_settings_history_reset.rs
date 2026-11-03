//! `settings_history.reset`: the entry records a reset to the default (the stored row was
//! deleted), not a new stored value.
//!
//! A reset deletes the `settings` row, so `new_value` (NOT NULL) keeps holding the public
//! JSON of the value that applies afterwards (the default) and this flag says why. The
//! history is also the only record of the version a deleted row had: the next row of that
//! key continues after the highest history version, so a client holding a version from
//! before the reset can never match a later row (no ABA on the optimistic lock).
//!
//! SQLite cannot add a CHECK to an existing table; the column is written only by the
//! settings service.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::boolean;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum SettingsHistory {
    Table,
    Reset,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let add = Table::alter()
            .table(SettingsHistory::Table)
            .add_column(boolean(SettingsHistory::Reset).default(false))
            .to_owned();
        manager.alter_table(add).await
    }
}
