//! The client keeps every option it was sent until it receives that key again (empty =
//! delete), so the server must remember what it sent, across restarts too. No foreign key:
//! deleting a device does not reset its client.

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
