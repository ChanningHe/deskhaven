//! Device groups (04-data-model §4). Membership is `devices.device_group_id`, whose foreign
//! key is added by the next migration.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_pk_auto, text};

use super::{enum_check, unique_index};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum DeviceGroups {
    Table,
    Id,
    Name,
    Note,
    ManagedBy,
    CreatedAt,
    UpdatedAt,
}

const MANAGED_BY: &[&str] = &["ui", "config"];

fn device_groups_table() -> TableCreateStatement {
    Table::create()
        .table(DeviceGroups::Table)
        .col(big_pk_auto(DeviceGroups::Id))
        .col(text(DeviceGroups::Name))
        .col(text(DeviceGroups::Note).default(""))
        .col(text(DeviceGroups::ManagedBy).default("ui"))
        .col(big_integer(DeviceGroups::CreatedAt))
        .col(big_integer(DeviceGroups::UpdatedAt))
        .check(enum_check(
            "ck_device_groups_managed_by",
            DeviceGroups::ManagedBy,
            MANAGED_BY,
        ))
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(device_groups_table()).await?;
        let unique_name = unique_index(
            "uq_device_groups_name",
            DeviceGroups::Table,
            DeviceGroups::Name,
        );
        manager.create_index(unique_name).await
    }
}
