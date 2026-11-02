//! Group writes. Each `*_guarded` variant runs a [`TxGuard`] inside its write transaction
//! (the console's audit row); the plain variants are the same writes without one.

use deskhaven_db::entity::{group, group_member, user};
use deskhaven_db::{begin_write, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, ModelTrait, TryInsertResult};

use super::{
    Group, GroupChange, GroupError, GroupService, GroupUpdate, ManagedBy, Membership, NewGroup,
    find_user, missing_reference, name_conflict, validate_name, validate_note, writable_group,
};
use crate::addressbooks;
use crate::console_audit::TxGuard;
use crate::strategies;

impl GroupService {
    pub async fn create(&self, new: NewGroup) -> Result<Group, GroupError> {
        self.create_guarded(new, TxGuard::none()).await
    }

    /// [`create`](Self::create), with `guard` deciding on the new group (no `before`).
    pub async fn create_guarded(
        &self,
        new: NewGroup,
        guard: TxGuard<Group>,
    ) -> Result<Group, GroupError> {
        let name = validate_name(&new.name)?;
        let note = validate_note(&new.note)?;
        self.detached(move |svc| async move {
            let now = time::now();
            let model = group::ActiveModel {
                name: Set(name),
                note: Set(note),
                managed_by: Set(ManagedBy::Ui),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            };
            let tx = begin_write(&svc.db).await?;
            let created: Group = model.insert(&tx).await.map_err(name_conflict)?.into();
            guard.check(&tx, None, Some(&created)).await?;
            tx.commit().await?;
            Ok(created)
        })
        .await
    }

    pub async fn update(&self, id: i64, update: GroupUpdate) -> Result<Group, GroupError> {
        self.update_guarded(id, update, TxGuard::none()).await
    }

    /// [`update`](Self::update), with `guard` deciding on the group before and after.
    pub async fn update_guarded(
        &self,
        id: i64,
        update: GroupUpdate,
        guard: TxGuard<Group>,
    ) -> Result<Group, GroupError> {
        let name = update.name.as_deref().map(validate_name).transpose()?;
        let note = update.note.as_deref().map(validate_note).transpose()?;
        self.detached(move |svc| async move {
            let tx = begin_write(&svc.db).await?;
            let current = writable_group(&tx, id).await?;
            let before = Group::from(current.clone());
            let mut changes: group::ActiveModel = current.into();
            if let Some(name) = name {
                changes.name = Set(name);
            }
            if let Some(note) = note {
                changes.note = Set(note);
            }
            changes.updated_at = Set(time::now());
            let updated: Group = changes.update(&tx).await.map_err(name_conflict)?.into();
            guard.check(&tx, Some(&before), Some(&updated)).await?;
            tx.commit().await?;
            Ok(updated)
        })
        .await
    }

    /// Deletes the group, its memberships (cascade), the address book shares naming it and
    /// its strategy assignment (same transaction: neither has a foreign key to the group).
    pub async fn delete(&self, id: i64) -> Result<(), GroupError> {
        self.delete_guarded(id, TxGuard::none()).await
    }

    /// [`delete`](Self::delete), with `guard` deciding on the group (no `after`).
    pub async fn delete_guarded(&self, id: i64, guard: TxGuard<Group>) -> Result<(), GroupError> {
        self.detached(move |svc| async move {
            let tx = begin_write(&svc.db).await?;
            let current = writable_group(&tx, id).await?;
            guard
                .check(&tx, Some(&current.clone().into()), None)
                .await?;
            addressbooks::forget_group(&tx, id).await?;
            strategies::forget_group(&tx, id).await?;
            current.delete(&tx).await?;
            tx.commit().await?;
            svc.committed(GroupChange::Deleted { group_id: id });
            Ok(())
        })
        .await
    }

    /// Adds `user_id` to the group; already a member is not an error (and reaches no hook).
    pub async fn add_member(&self, group_id: i64, user_id: i64) -> Result<(), GroupError> {
        self.add_member_guarded(group_id, user_id, TxGuard::none())
            .await
    }

    /// [`add_member`](Self::add_member), with `guard` deciding on the membership: `before`
    /// is set when the user already was a member.
    pub async fn add_member_guarded(
        &self,
        group_id: i64,
        user_id: i64,
        guard: TxGuard<Membership>,
    ) -> Result<(), GroupError> {
        self.detached(move |svc| async move {
            let tx = begin_write(&svc.db).await?;
            let group = writable_group(&tx, group_id).await?;
            let user = find_user(&tx, user_id).await?;
            let member = group_member::ActiveModel {
                group_id: Set(group_id),
                user_id: Set(user_id),
                created_at: Set(time::now()),
            };
            let inserted = group_member::Entity::insert(member)
                .on_conflict_do_nothing_on([
                    group_member::Column::GroupId,
                    group_member::Column::UserId,
                ])
                .exec_without_returning(&tx)
                .await
                .map_err(missing_reference)?;
            let added = matches!(inserted, TryInsertResult::Inserted(n) if n > 0);
            let membership = Membership::of(group, &user);
            let before = (!added).then_some(&membership);
            guard.check(&tx, before, Some(&membership)).await?;
            tx.commit().await?;
            if added {
                svc.committed(GroupChange::MembershipChanged { group_id, user_id });
            }
            Ok(())
        })
        .await
    }

    /// Removes `user_id` from the group; not a member is not an error (and reaches no hook).
    pub async fn remove_member(&self, group_id: i64, user_id: i64) -> Result<(), GroupError> {
        self.remove_member_guarded(group_id, user_id, TxGuard::none())
            .await
    }

    /// [`remove_member`](Self::remove_member), with `guard` deciding on the membership:
    /// `before` is set when the user was a member. An unknown user is not a member.
    pub async fn remove_member_guarded(
        &self,
        group_id: i64,
        user_id: i64,
        guard: TxGuard<Membership>,
    ) -> Result<(), GroupError> {
        self.detached(move |svc| async move {
            let tx = begin_write(&svc.db).await?;
            let group = writable_group(&tx, group_id).await?;
            let removed = group_member::Entity::delete_by_id((group_id, user_id))
                .exec(&tx)
                .await?;
            let removed = removed.rows_affected > 0;
            let user = user::Entity::find_by_id(user_id).one(&tx).await?;
            let membership = Membership {
                group: group.into(),
                user_id,
                username: user.map(|u| u.username),
            };
            guard
                .check(&tx, removed.then_some(&membership), None)
                .await?;
            tx.commit().await?;
            if removed {
                svc.committed(GroupChange::MembershipChanged { group_id, user_id });
            }
            Ok(())
        })
        .await
    }
}
