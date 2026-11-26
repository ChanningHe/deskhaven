//! The role plays no part: an admin's client sees what a member of the same groups sees.

use deskhaven_db::entity::{device, group_member, user};
use sea_orm::sea_query::SelectStatement;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect, QueryTrait};

fn teammates(viewer: i64) -> SelectStatement {
    let own_groups = group_member::Entity::find()
        .select_only()
        .column(group_member::Column::GroupId)
        .filter(group_member::Column::UserId.eq(viewer))
        .into_query();
    group_member::Entity::find()
        .select_only()
        .column(group_member::Column::UserId)
        .filter(group_member::Column::GroupId.in_subquery(own_groups))
        .into_query()
}

pub(super) fn visible_users(viewer: i64) -> Condition {
    Condition::any()
        .add(user::Column::Id.eq(viewer))
        .add(user::Column::Id.in_subquery(teammates(viewer)))
}

pub(super) fn visible_devices(viewer: i64) -> Condition {
    Condition::any()
        .add(device::Column::OwnerUserId.eq(viewer))
        .add(device::Column::OwnerUserId.in_subquery(teammates(viewer)))
}
