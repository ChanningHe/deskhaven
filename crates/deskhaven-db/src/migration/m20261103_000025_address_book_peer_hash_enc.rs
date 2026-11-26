//! The personal-book hash is not a verifier but the salted password itself: the client sends
//! it as the connection password, so whoever reads it can log in (pass-the-hash). Plain values
//! cannot be sealed in SQL and no deployment held address book data yet, so the column is
//! dropped and the new one starts empty. Separate statements: SQLite accepts one change per
//! `ALTER TABLE`.

use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::text_null;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum AddressBookPeers {
    Table,
    Hash,
    HashEnc,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let drop = Table::alter()
            .table(AddressBookPeers::Table)
            .drop_column(AddressBookPeers::Hash)
            .to_owned();
        manager.alter_table(drop).await?;
        let add = Table::alter()
            .table(AddressBookPeers::Table)
            .add_column(text_null(AddressBookPeers::HashEnc))
            .to_owned();
        manager.alter_table(add).await
    }
}
