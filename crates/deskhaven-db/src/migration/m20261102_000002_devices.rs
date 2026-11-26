//! `owner_user_id` / `device_group_id` get their foreign keys in migration 000021: their
//! tables do not exist yet. `uuid` / `pk` are the raw RegisterPk bytes: opaque, compared byte
//! for byte.

use super::{enum_check, unique_index};
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, blob, text, text_null,
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Devices {
    Table,
    Id,
    RustdeskId,
    Uuid,
    Pk,
    Hostname,
    Username,
    Os,
    Cpu,
    Memory,
    Version,
    SysinfoHash,
    OwnerUserId,
    DeviceGroupId,
    Note,
    Status,
    DeployedAt,
    FirstSeenAt,
    LastSeenAt,
    LastIp,
    RegisterIp,
    RegisterAt,
    CreatedAt,
    UpdatedAt,
}

const STATUSES: [&str; 2] = ["active", "disabled"];
const DEFAULT_STATUS: &str = "active";

fn devices_table() -> TableCreateStatement {
    Table::create()
        .table(Devices::Table)
        .col(big_pk_auto(Devices::Id))
        .col(text(Devices::RustdeskId))
        .col(blob(Devices::Uuid))
        .col(blob(Devices::Pk))
        .col(text_null(Devices::Hostname))
        .col(text_null(Devices::Username))
        .col(text_null(Devices::Os))
        .col(text_null(Devices::Cpu))
        .col(text_null(Devices::Memory))
        .col(text_null(Devices::Version))
        .col(text_null(Devices::SysinfoHash))
        .col(big_integer_null(Devices::OwnerUserId))
        .col(big_integer_null(Devices::DeviceGroupId))
        .col(text_null(Devices::Note))
        .col(text(Devices::Status).default(DEFAULT_STATUS))
        .col(big_integer_null(Devices::DeployedAt))
        .col(big_integer(Devices::FirstSeenAt))
        .col(big_integer(Devices::LastSeenAt))
        .col(text(Devices::LastIp))
        .col(text(Devices::RegisterIp))
        .col(big_integer(Devices::RegisterAt))
        .col(big_integer(Devices::CreatedAt))
        .col(big_integer(Devices::UpdatedAt))
        .check(enum_check("ck_devices_status", Devices::Status, &STATUSES))
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(devices_table()).await?;
        let rustdesk_id = unique_index(
            "ux_devices_rustdesk_id",
            Devices::Table,
            Devices::RustdeskId,
        );
        manager.create_index(rustdesk_id).await
    }
}
