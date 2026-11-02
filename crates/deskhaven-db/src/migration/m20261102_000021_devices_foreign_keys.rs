//! Foreign keys `devices.owner_user_id -> users(id)` and `devices.device_group_id ->
//! device_groups(id)`, both `ON DELETE SET NULL`: deleting the user or the group leaves the
//! device registered but unassigned (04-data-model §4: NULL = unassigned).
//!
//! The columns predate their target tables (migration 000002 carries no FK), so existing rows
//! may only hold references that never pointed anywhere; they are cleared first rather than
//! failing the whole upgrade.
//!
//! PostgreSQL adds the constraints in place. SQLite cannot add a foreign key to an existing
//! table, so the table is rebuilt: create the new shape, copy the rows, drop the old table,
//! rename. Nothing references `devices`, so the drop needs no `defer_foreign_keys`.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{
    big_integer, big_integer_null, big_pk_auto, blob, text, text_null,
};

use super::{enum_check, foreign_key, index, unique_index};

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

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum DeviceGroups {
    Table,
    Id,
}

/// Every column of `devices`, in table order (migration 000002).
const COLUMNS: [&str; 23] = [
    "id",
    "rustdesk_id",
    "uuid",
    "pk",
    "hostname",
    "username",
    "os",
    "cpu",
    "memory",
    "version",
    "sysinfo_hash",
    "owner_user_id",
    "device_group_id",
    "note",
    "status",
    "deployed_at",
    "first_seen_at",
    "last_seen_at",
    "last_ip",
    "register_ip",
    "register_at",
    "created_at",
    "updated_at",
];
const STATUSES: [&str; 2] = ["active", "disabled"];
const DEFAULT_STATUS: &str = "active";
const REBUILT: &str = "devices_new";

/// `col` if it names an existing row of `target`, else NULL.
fn valid_reference(col: &str, target: &str) -> String {
    format!("CASE WHEN {col} IN (SELECT id FROM {target}) THEN {col} END")
}

fn owner_reference() -> String {
    valid_reference("owner_user_id", "users")
}

fn group_reference() -> String {
    valid_reference("device_group_id", "device_groups")
}

/// `devices` as migration 000002 created it, plus the two foreign keys, under `name`.
fn devices_table(name: DynIden) -> TableCreateStatement {
    Table::create()
        .table(name.clone())
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
        .foreign_key(&mut owner_fk(name.clone()))
        .foreign_key(&mut group_fk(name))
        .to_owned()
}

fn owner_fk(table: DynIden) -> ForeignKeyCreateStatement {
    foreign_key(
        "fk_devices_owner_user",
        (table, Devices::OwnerUserId.into_iden()),
        (Users::Table.into_iden(), Users::Id.into_iden()),
        ForeignKeyAction::SetNull,
    )
}

fn group_fk(table: DynIden) -> ForeignKeyCreateStatement {
    foreign_key(
        "fk_devices_device_group",
        (table, Devices::DeviceGroupId.into_iden()),
        (
            DeviceGroups::Table.into_iden(),
            DeviceGroups::Id.into_iden(),
        ),
        ForeignKeyAction::SetNull,
    )
}

/// SQLite: rebuild the table with the foreign keys and re-create what the drop took along.
async fn rebuild_sqlite(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(devices_table(Alias::new(REBUILT).into_iden()))
        .await?;
    let insert_columns = COLUMNS.join(", ");
    let select_columns = COLUMNS
        .iter()
        .map(|col| match *col {
            "owner_user_id" => owner_reference(),
            "device_group_id" => group_reference(),
            other => other.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let copy =
        format!("INSERT INTO {REBUILT} ({insert_columns}) SELECT {select_columns} FROM devices");
    manager.get_connection().execute_unprepared(&copy).await?;
    manager
        .drop_table(Table::drop().table(Devices::Table).to_owned())
        .await?;
    manager
        .rename_table(
            Table::rename()
                .table(Alias::new(REBUILT), Devices::Table)
                .to_owned(),
        )
        .await?;
    // The old table's index went with it.
    let rustdesk_id = unique_index(
        "ux_devices_rustdesk_id",
        Devices::Table,
        Devices::RustdeskId,
    );
    manager.create_index(rustdesk_id).await
}

/// PostgreSQL: clear unusable references, then add the constraints in place.
async fn alter_postgres(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let clear = format!(
        "UPDATE devices SET owner_user_id = {}, device_group_id = {}",
        owner_reference(),
        group_reference()
    );
    manager.get_connection().execute_unprepared(&clear).await?;
    manager
        .create_foreign_key(owner_fk(Devices::Table.into_iden()))
        .await?;
    manager
        .create_foreign_key(group_fk(Devices::Table.into_iden()))
        .await
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        match manager.get_database_backend() {
            DbBackend::Sqlite => rebuild_sqlite(manager).await?,
            _ => alter_postgres(manager).await?,
        }
        // PostgreSQL does not index FK columns itself; both serve the SET NULL actions.
        let by_owner = index(
            "idx_devices_owner_user_id",
            Devices::Table,
            Devices::OwnerUserId,
        );
        manager.create_index(by_owner).await?;
        let by_group = index(
            "idx_devices_device_group_id",
            Devices::Table,
            Devices::DeviceGroupId,
        );
        manager.create_index(by_group).await
    }
}
