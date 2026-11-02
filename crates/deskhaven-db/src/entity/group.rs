//! `groups`: user groups (membership in `group_members`).

use sea_orm::entity::prelude::*;

use super::managed_by::ManagedBy;
use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "groups")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    pub note: String,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::group_member::Entity")]
    GroupMember,
}

impl Related<super::group_member::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GroupMember.def()
    }
}

impl Related<super::user::Entity> for Entity {
    fn to() -> RelationDef {
        super::group_member::Relation::User.def()
    }
    fn via() -> Option<RelationDef> {
        Some(super::group_member::Relation::Group.def().rev())
    }
}

impl ActiveModelBehavior for ActiveModel {}
