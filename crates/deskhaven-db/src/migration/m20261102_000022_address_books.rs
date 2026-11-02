//! Address books, their shares, peers and tags (04-data-model §5).
//!
//! A user has at most one `personal` book: a partial unique index over the owner, which only
//! covers `kind = 'personal'` rows (PostgreSQL and SQLite both support partial indexes). A
//! book is owned by exactly one user and is deleted with them. Shares point at a user or a
//! group through `(subject_type, subject_id)`, so `subject_id` has no foreign key and the
//! address book service removes shares of deleted subjects. Peers have no foreign key to
//! `devices`: a book may list devices this server has never seen. Text columns the client
//! echoes back default to the empty string; `hash` (personal books) and `password_enc`
//! (shared books) are the only optional ones, and which of the two is set is service logic
//! because `kind` lives in another table.

use sea_orm::DbBackend;
use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::{big_integer, big_pk_auto, boolean, integer, text, text_null};

use super::{bool_checks, enum_check, foreign_key, index, unique_index};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum AddressBooks {
    Table,
    Id,
    Guid,
    Kind,
    OwnerUserId,
    Name,
    Note,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum AddressBookShares {
    Table,
    AddressBookId,
    SubjectType,
    SubjectId,
    Rule,
}

#[derive(DeriveIden)]
enum AddressBookPeers {
    Table,
    AddressBookId,
    RustdeskId,
    Alias,
    Note,
    Tags,
    Hash,
    PasswordEnc,
    Username,
    Hostname,
    Platform,
    ForceAlwaysRelay,
    RdpPort,
    RdpUsername,
    LoginName,
    Extra,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum AddressBookTags {
    Table,
    AddressBookId,
    Name,
    Color,
}

const KINDS: &[&str] = &["personal", "shared"];
const SUBJECT_TYPES: &[&str] = &["user", "group"];
/// 1 read-only, 2 read-write, 3 full control.
const RULES: [i32; 3] = [1, 2, 3];
const EMPTY_JSON_OBJECT: &str = "{}";
const EMPTY_JSON_ARRAY: &str = "[]";

fn address_books_table() -> TableCreateStatement {
    Table::create()
        .table(AddressBooks::Table)
        .col(big_pk_auto(AddressBooks::Id))
        .col(text(AddressBooks::Guid))
        .col(text(AddressBooks::Kind))
        .col(big_integer(AddressBooks::OwnerUserId))
        .col(text(AddressBooks::Name))
        .col(text(AddressBooks::Note).default(""))
        .col(big_integer(AddressBooks::CreatedAt))
        .col(big_integer(AddressBooks::UpdatedAt))
        .check(enum_check(
            "ck_address_books_kind",
            AddressBooks::Kind,
            KINDS,
        ))
        .foreign_key(&mut foreign_key(
            "fk_address_books_owner",
            (AddressBooks::Table, AddressBooks::OwnerUserId),
            (Users::Table, Users::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

fn shares_table() -> TableCreateStatement {
    Table::create()
        .table(AddressBookShares::Table)
        .col(big_integer(AddressBookShares::AddressBookId))
        .col(text(AddressBookShares::SubjectType))
        .col(big_integer(AddressBookShares::SubjectId))
        .col(integer(AddressBookShares::Rule))
        .primary_key(
            Index::create()
                .col(AddressBookShares::AddressBookId)
                .col(AddressBookShares::SubjectType)
                .col(AddressBookShares::SubjectId),
        )
        .check(enum_check(
            "ck_address_book_shares_subject_type",
            AddressBookShares::SubjectType,
            SUBJECT_TYPES,
        ))
        .check(Check::named(
            "ck_address_book_shares_rule",
            Expr::col(AddressBookShares::Rule).is_in(RULES),
        ))
        .foreign_key(&mut foreign_key(
            "fk_address_book_shares_book",
            (AddressBookShares::Table, AddressBookShares::AddressBookId),
            (AddressBooks::Table, AddressBooks::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

fn peers_table(backend: DbBackend) -> TableCreateStatement {
    let mut table = Table::create()
        .table(AddressBookPeers::Table)
        .col(big_integer(AddressBookPeers::AddressBookId))
        .col(text(AddressBookPeers::RustdeskId))
        .col(text(AddressBookPeers::Alias).default(""))
        .col(text(AddressBookPeers::Note).default(""))
        .col(text(AddressBookPeers::Tags).default(EMPTY_JSON_ARRAY))
        .col(text_null(AddressBookPeers::Hash))
        .col(text_null(AddressBookPeers::PasswordEnc))
        .col(text(AddressBookPeers::Username).default(""))
        .col(text(AddressBookPeers::Hostname).default(""))
        .col(text(AddressBookPeers::Platform).default(""))
        .col(boolean(AddressBookPeers::ForceAlwaysRelay).default(false))
        .col(text(AddressBookPeers::RdpPort).default(""))
        .col(text(AddressBookPeers::RdpUsername).default(""))
        .col(text(AddressBookPeers::LoginName).default(""))
        .col(text(AddressBookPeers::Extra).default(EMPTY_JSON_OBJECT))
        .col(big_integer(AddressBookPeers::UpdatedAt))
        .primary_key(
            Index::create()
                .col(AddressBookPeers::AddressBookId)
                .col(AddressBookPeers::RustdeskId),
        )
        .foreign_key(&mut foreign_key(
            "fk_address_book_peers_book",
            (AddressBookPeers::Table, AddressBookPeers::AddressBookId),
            (AddressBooks::Table, AddressBooks::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned();
    bool_checks(
        &mut table,
        backend,
        "address_book_peers",
        &["force_always_relay"],
    );
    table
}

fn tags_table() -> TableCreateStatement {
    Table::create()
        .table(AddressBookTags::Table)
        .col(big_integer(AddressBookTags::AddressBookId))
        .col(text(AddressBookTags::Name))
        .col(big_integer(AddressBookTags::Color))
        .primary_key(
            Index::create()
                .col(AddressBookTags::AddressBookId)
                .col(AddressBookTags::Name),
        )
        .foreign_key(&mut foreign_key(
            "fk_address_book_tags_book",
            (AddressBookTags::Table, AddressBookTags::AddressBookId),
            (AddressBooks::Table, AddressBooks::Id),
            ForeignKeyAction::Cascade,
        ))
        .to_owned()
}

/// At most one personal book per user. Shared books are not constrained.
fn personal_owner_index() -> IndexCreateStatement {
    Index::create()
        .name("uq_address_books_personal_owner")
        .table(AddressBooks::Table)
        .col(AddressBooks::OwnerUserId)
        .unique()
        .and_where(Expr::col(AddressBooks::Kind).eq("personal"))
        .to_owned()
}

fn indexes() -> Vec<IndexCreateStatement> {
    vec![
        unique_index(
            "uq_address_books_guid",
            AddressBooks::Table,
            AddressBooks::Guid,
        ),
        personal_owner_index(),
        // Shared books of a user, and the owner-delete cascade.
        index(
            "idx_address_books_owner_user_id",
            AddressBooks::Table,
            AddressBooks::OwnerUserId,
        ),
        // The primary key leads with the book; this one answers "books shared with me".
        Index::create()
            .name("idx_address_book_shares_subject")
            .table(AddressBookShares::Table)
            .col(AddressBookShares::SubjectType)
            .col(AddressBookShares::SubjectId)
            .to_owned(),
    ]
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        let tables = [
            address_books_table(),
            shares_table(),
            peers_table(backend),
            tags_table(),
        ];
        for table in tables {
            manager.create_table(table).await?;
        }
        for statement in indexes() {
            manager.create_index(statement).await?;
        }
        Ok(())
    }
}
