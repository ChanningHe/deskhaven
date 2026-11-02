//! `strategy_assignments`: which strategy applies to which target (at most one per target).
//!
//! `target_id` points at `devices.id`, `device_groups.id`, `users.id` or `groups.id`
//! depending on `target_type`, so it has no foreign key; the strategy service removes
//! assignments of deleted targets. The `global` target has no id.

use sea_orm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum TargetType {
    #[sea_orm(string_value = "device")]
    Device,
    #[sea_orm(string_value = "device_group")]
    DeviceGroup,
    #[sea_orm(string_value = "user")]
    User,
    #[sea_orm(string_value = "group")]
    Group,
    #[sea_orm(string_value = "global")]
    Global,
}

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "strategy_assignments")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub strategy_id: i64,
    pub target_type: TargetType,
    /// `None` exactly for [`TargetType::Global`].
    pub target_id: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::strategy::Entity",
        from = "Column::StrategyId",
        to = "super::strategy::Column::Id",
        on_delete = "Cascade"
    )]
    Strategy,
}

impl Related<super::strategy::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Strategy.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
