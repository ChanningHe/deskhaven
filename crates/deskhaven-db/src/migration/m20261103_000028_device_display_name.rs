//! Names an administrator gave the device; NULL until set. Sysinfo never writes them, unlike
//! `hostname` / `username`.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::text_null;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Devices {
    Table,
    DisplayName,
    DisplayUsername,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // One column per statement: SQLite's ALTER TABLE adds a single column.
        for column in [Devices::DisplayName, Devices::DisplayUsername] {
            let add = Table::alter()
                .table(Devices::Table)
                .add_column(text_null(column))
                .to_owned();
            manager.alter_table(add).await?;
        }
        Ok(())
    }
}
