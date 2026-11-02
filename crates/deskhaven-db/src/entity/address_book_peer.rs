//! `address_book_peers`: a device entry of an address book, keyed by `(book, rustdesk_id)`.
//!
//! No foreign key to `devices`: entries may name devices this server has never seen.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "address_book_peers")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub address_book_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub rustdesk_id: String,
    pub alias: String,
    pub note: String,
    /// JSON array of tag names.
    pub tags: String,
    /// Personal books: password hash computed by the client.
    pub hash: Option<String>,
    /// Shared books: encrypted password.
    pub password_enc: Option<String>,
    pub username: String,
    pub hostname: String,
    pub platform: String,
    pub force_always_relay: bool,
    pub rdp_port: String,
    pub rdp_username: String,
    pub login_name: String,
    /// JSON object: fields this server does not know, kept verbatim for forward compatibility.
    pub extra: String,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::address_book::Entity",
        from = "Column::AddressBookId",
        to = "super::address_book::Column::Id",
        on_delete = "Cascade"
    )]
    AddressBook,
}

impl Related<super::address_book::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::AddressBook.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
