//! `devices.display_name` / `devices.display_username` (M2-19): the names an administrator
//! gave the device (`rustdesk --assign --device_name / --device_username`, later the
//! console). NULL until set. The device's own sysinfo never writes them, unlike `hostname` /
//! `username`, which it overwrites with every changed upload.

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
