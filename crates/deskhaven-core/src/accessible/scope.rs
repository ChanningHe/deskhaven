//! The P0 visibility scope (06 §4.2) as SQL conditions: a viewer reaches themselves, the
//! users who share a group with them, and the devices those users own. The role plays no
//! part: an admin's client sees what an ordinary member of the same groups sees (the
//! console is where everything is managed). P1 replaces these two conditions with the ACL
//! rules; the queries around them stay.

use deskhaven_db::entity::{device, group_member, user};
use sea_orm::sea_query::SelectStatement;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect, QueryTrait};

/// Ids of the users in at least one group of `viewer` (the viewer too, when in any group).
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

/// The users `viewer` can see: themselves and their teammates.
pub(super) fn visible_users(viewer: i64) -> Condition {
    Condition::any()
        .add(user::Column::Id.eq(viewer))
        .add(user::Column::Id.in_subquery(teammates(viewer)))
}

/// The devices `viewer` can see: those owned by a [`visible_users`] user. A device without
/// an owner is never visible.
pub(super) fn visible_devices(viewer: i64) -> Condition {
    Condition::any()
        .add(device::Column::OwnerUserId.eq(viewer))
        .add(device::Column::OwnerUserId.in_subquery(teammates(viewer)))
}
