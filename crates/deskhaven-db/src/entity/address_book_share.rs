//! `address_book_shares`: a shared book granted to a user or a group with a rule.
//!
//! `subject_id` points at `users.id` or `groups.id` depending on `subject_type`, so it has no
//! foreign key; the address book service removes shares of deleted subjects.

use sea_orm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum SubjectType {
    #[sea_orm(string_value = "user")]
    User,
    #[sea_orm(string_value = "group")]
    Group,
}

/// Access level, stored as the number clients use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "i32", db_type = "Integer")]
pub enum Rule {
    #[sea_orm(num_value = 1)]
    ReadOnly,
    #[sea_orm(num_value = 2)]
    ReadWrite,
    #[sea_orm(num_value = 3)]
    FullControl,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "address_book_shares")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub address_book_id: i64,
    #[sea_orm(primary_key, auto_increment = false)]
    pub subject_type: SubjectType,
    #[sea_orm(primary_key, auto_increment = false)]
    pub subject_id: i64,
    pub rule: Rule,
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
