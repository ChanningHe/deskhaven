//! `device_strategy_keys` (M2-14): per device, the strategy option keys the heartbeat last
//! sent with a value. The client keeps every option it was ever sent until it receives that
//! key again (empty = delete), so the server must remember what it sent to delete what later
//! disappears from the effective strategy. Kept in the database because a restart must not
//! forget it: a device offline across a removal and a restart would otherwise keep the
//! removed option for good.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, text};

use super::foreign_key;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum DeviceStrategyKeys {
    Table,
    DeviceId,
    OptionKeys,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Devices {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let table = Table::create()
            .table(DeviceStrategyKeys::Table)
            .col(big_integer(DeviceStrategyKeys::DeviceId).primary_key())
            .col(text(DeviceStrategyKeys::OptionKeys))
            .col(big_integer(DeviceStrategyKeys::UpdatedAt))
            .foreign_key(&mut foreign_key(
                "fk_device_strategy_keys_device",
                (DeviceStrategyKeys::Table, DeviceStrategyKeys::DeviceId),
                (Devices::Table, Devices::Id),
                ForeignKeyAction::Cascade,
            ))
            .to_owned();
        manager.create_table(table).await
    }
}
