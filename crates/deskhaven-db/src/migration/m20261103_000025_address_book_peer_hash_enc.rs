//! `address_book_peers.hash` becomes `hash_enc`: the personal-book password hash is sealed
//! at rest like the shared-book password.
//!
//! The hash the client stores in a personal book is not a verifier but the salted password
//! itself: the client sends it as the connection password (`src/client.rs:4805-4827`), so
//! whoever reads it can log in to that device (pass-the-hash). Migration 000022 stored it
//! in clear (its header calls it "the password hash"); from here on it is sealed with the
//! master key (AAD context `address_book_peers.hash_enc:<book id>:<rustdesk id>`, see
//! `deskhaven_core::addressbooks`).
//!
//! Plain `hash` values cannot be sealed in SQL, and no deployment holds address book data
//! yet, so the column is dropped and the new one starts empty. Both steps are separate
//! statements: SQLite accepts one change per `ALTER TABLE` (`DROP COLUMN` needs SQLite
//! 3.35, the column has no index or constraint).

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
