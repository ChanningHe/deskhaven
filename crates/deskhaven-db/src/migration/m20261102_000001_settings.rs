//! Runtime settings store and its change history (05-config §3.2).
//! `updated_by` will reference `users(id)` once that table exists; no FK yet.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_integer_null, big_pk_auto, text, text_null};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Settings {
    Table,
    Key,
    Value,
    Version,
    UpdatedBy,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum SettingsHistory {
    Table,
    Id,
    Key,
    OldValue,
    NewValue,
    Version,
    UpdatedBy,
    UpdatedAt,
    Reason,
}

fn settings_table() -> TableCreateStatement {
    Table::create()
        .table(Settings::Table)
        .col(text(Settings::Key).primary_key())
        .col(text(Settings::Value))
        .col(big_integer(Settings::Version))
        .col(big_integer_null(Settings::UpdatedBy))
        .col(big_integer(Settings::UpdatedAt))
        .to_owned()
}

fn settings_history_table() -> TableCreateStatement {
    Table::create()
        .table(SettingsHistory::Table)
        .col(big_pk_auto(SettingsHistory::Id))
        .col(text(SettingsHistory::Key))
        .col(text_null(SettingsHistory::OldValue))
        .col(text(SettingsHistory::NewValue))
        .col(big_integer(SettingsHistory::Version))
        .col(big_integer_null(SettingsHistory::UpdatedBy))
        .col(big_integer(SettingsHistory::UpdatedAt))
        .col(text_null(SettingsHistory::Reason))
        .to_owned()
}

fn settings_history_key_index() -> IndexCreateStatement {
    Index::create()
        .name("idx_settings_history_key_updated_at")
        .table(SettingsHistory::Table)
        .col(SettingsHistory::Key)
        .col(SettingsHistory::UpdatedAt)
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(settings_table()).await?;
        manager.create_table(settings_history_table()).await?;
        manager.create_index(settings_history_key_index()).await
    }
}
