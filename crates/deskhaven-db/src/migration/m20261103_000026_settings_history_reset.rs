//! A reset deletes the `settings` row, so `new_value` keeps the value that applies afterwards
//! and `reset` says why. The history is the only record of a deleted row's version: the next
//! row continues after the highest history version, so the optimistic lock has no ABA.
//! `UNIQUE(key, version)` backstops that; existing data cannot violate it (every write bumped
//! the version by one and added one entry). SQLite cannot add a CHECK to an existing table.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::boolean;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum SettingsHistory {
    Table,
    Reset,
    Key,
    Version,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let add = Table::alter()
            .table(SettingsHistory::Table)
            .add_column(boolean(SettingsHistory::Reset).default(false))
            .to_owned();
        manager.alter_table(add).await?;
        let unique = Index::create()
            .name("uq_settings_history_key_version")
            .table(SettingsHistory::Table)
            .col(SettingsHistory::Key)
            .col(SettingsHistory::Version)
            .unique()
            .to_owned();
        manager.create_index(unique).await
    }
}
