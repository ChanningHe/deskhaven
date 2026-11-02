//! `address_books`: a user's personal book (at most one) or a shared book.

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum Kind {
    #[sea_orm(string_value = "personal")]
    Personal,
    #[sea_orm(string_value = "shared")]
    Shared,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "address_books")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// UUIDv7 shown to clients.
    pub guid: String,
    pub kind: Kind,
    /// The book is deleted with its owner.
    pub owner_user_id: i64,
    pub name: String,
    pub note: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::user::Entity",
        from = "Column::OwnerUserId",
        to = "super::user::Column::Id",
        on_delete = "Cascade"
    )]
    User,
    #[sea_orm(has_many = "super::address_book_share::Entity")]
    Share,
    #[sea_orm(has_many = "super::address_book_peer::Entity")]
    Peer,
    #[sea_orm(has_many = "super::address_book_tag::Entity")]
    Tag,
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
}

impl Related<super::address_book_share::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Share.def()
    }
}

impl Related<super::address_book_peer::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Peer.def()
    }
}

impl Related<super::address_book_tag::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Tag.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
