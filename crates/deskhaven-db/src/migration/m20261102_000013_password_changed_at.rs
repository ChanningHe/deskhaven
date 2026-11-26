//! Session creation re-checks this credential stamp in its own transaction, so a session minted
//! from the old password cannot outlive a password change. No CHECK ties it to `auth_source`:
//! SQLite cannot add constraints to an existing table.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::big_integer_null;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    PasswordChangedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let add = Table::alter()
            .table(Users::Table)
            .add_column(big_integer_null(Users::PasswordChangedAt))
            .to_owned();
        manager.alter_table(add).await
    }
}
