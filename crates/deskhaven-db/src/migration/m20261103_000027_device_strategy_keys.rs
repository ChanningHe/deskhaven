//! `device_strategy_keys` (M2-14): per RustDesk id, the strategy option keys the heartbeat
//! last sent with a value. The client keeps every option it was ever sent until it receives
//! that key again (empty = delete), so the server must remember what it sent to delete what
//! later disappears from the effective strategy.
//!
//! Kept in the database because a restart must not forget it: a device offline across a
//! removal and a restart would otherwise keep the removed option for good. Keyed by RustDesk
//! id without a foreign key for the same reason: deleting a device does not reset its client,
//! which re-registers as a new `devices` row and still holds what it was sent.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, text};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum DeviceStrategyKeys {
    Table,
    RustdeskId,
    OptionKeys,
    UpdatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = Table::create()
            .table(DeviceStrategyKeys::Table)
            .col(text(DeviceStrategyKeys::RustdeskId).primary_key())
            .col(text(DeviceStrategyKeys::OptionKeys))
            .col(big_integer(DeviceStrategyKeys::UpdatedAt))
            .to_owned();
        manager.create_table(table).await
    }
}
