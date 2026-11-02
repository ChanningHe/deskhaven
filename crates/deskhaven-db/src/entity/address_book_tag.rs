//! `address_book_tags`: a tag name and its colour within an address book.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "address_book_tags")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub address_book_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub name: String,
    /// ARGB, as the client sends it (needs 32 bits, hence `BIGINT`).
    pub color: i64,
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
