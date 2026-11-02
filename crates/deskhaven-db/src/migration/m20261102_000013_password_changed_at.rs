//! `users.password_changed_at`: when the local password was last set (unix ms; NULL for
//! accounts without a local password).
//!
//! A login records it as the credential stamp it verified against; session creation
//! re-checks the stamp inside its own transaction, so a password changed between the
//! password check and the session insert cannot be outlived by a session minted from the old
//! password (06 §3.2). No CHECK ties it to `auth_source`: SQLite cannot add constraints to an
//! existing table, and the service is the only writer of passwords.

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
