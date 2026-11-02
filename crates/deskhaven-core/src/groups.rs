//! User groups and membership (04 §3).
//!
//! Groups with `managed_by = config` come from the bootstrap configuration: they can be read
//! here but every mutation (rename, delete, membership) is refused with
//! [`GroupError::ManagedByConfig`]. Membership changes are idempotent.

use std::future::Future;
use std::sync::{Arc, PoisonError, RwLock};

use deskhaven_db::entity::{group, group_member, user};
use deskhaven_db::time::TimestampMs;
use deskhaven_db::{begin_write, is_foreign_key_violation, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait,
    ModelTrait, QueryFilter, QueryOrder, QuerySelect, SqlErr, TryInsertResult,
};

pub use deskhaven_db::entity::managed_by::ManagedBy;

use crate::addressbooks;
use crate::strategies;
use crate::text::{self, NOTE_MAX_LEN};
use crate::users::User;

const NAME_MAX_LEN: usize = 64;
/// Name of the membership foreign key to `groups` (PostgreSQL names it in FK errors).
const FK_MEMBER_GROUP: &str = "fk_group_members_group";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub note: String,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<group::Model> for Group {
    fn from(m: group::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            note: m.note,
            managed_by: m.managed_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Input of [`GroupService::create`]; the group is managed in the console (`ui`).
#[derive(Clone, Debug)]
pub struct NewGroup {
    pub name: String,
    pub note: String,
}

impl NewGroup {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            note: String::new(),
        }
    }
}

/// Fields to change; `None` leaves a field as it is.
#[derive(Clone, Debug, Default)]
pub struct GroupUpdate {
    pub name: Option<String>,
    pub note: Option<String>,
}

/// Why a group operation failed. Variants are stable for API error mapping.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GroupError {
    #[error("group not found")]
    NotFound,

    #[error("user not found")]
    UserNotFound,

    #[error("{field} is already taken")]
    Conflict { field: &'static str },

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    /// The group is defined in the configuration file and read-only here.
    #[error("the group is managed by the configuration file")]
    ManagedByConfig,

    /// The operation's task panicked or the runtime is shutting down; whether it committed
    /// is unknown until the next read.
    #[error("group operation did not complete")]
    Aborted,

    #[error("group storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for GroupError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

/// A committed change that cached views of group membership must drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GroupChange {
    /// The group, its memberships and its strategy assignment are gone.
    Deleted { group_id: i64 },
    /// `add_member` / `remove_member` committed and changed a membership row (not sent when
    /// the user already was / was not a member).
    MembershipChanged { group_id: i64, user_id: i64 },
}

/// Called synchronously after each committed [`GroupChange`], before the writing call
/// returns. Must be fast and must not block or panic.
pub type GroupCommitHook = Arc<dyn Fn(&GroupChange) + Send + Sync>;

/// Manages groups and membership. Cheap to clone (clones share the hooks).
#[derive(Clone)]
pub struct GroupService {
    db: DatabaseConnection,
    hooks: Arc<RwLock<Vec<GroupCommitHook>>>,
}

impl GroupService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            hooks: Arc::default(),
        }
    }

    /// Calls `hook` after every later committed [`GroupChange`].
    pub fn register_commit_hook(&self, hook: GroupCommitHook) {
        self.hooks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hook);
    }

    fn committed(&self, change: GroupChange) {
        let hooks = self.hooks.read().unwrap_or_else(PoisonError::into_inner);
        for hook in hooks.iter() {
            hook(&change);
        }
    }

    pub async fn create(&self, new: NewGroup) -> Result<Group, GroupError> {
        let name = validate_name(&new.name)?;
        let note = validate_note(&new.note)?;
        let now = time::now();
        let model = group::ActiveModel {
            name: Set(name),
            note: Set(note),
            managed_by: Set(ManagedBy::Ui),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let created = model.insert(&self.db).await.map_err(name_conflict)?;
        Ok(created.into())
    }

    pub async fn get(&self, id: i64) -> Result<Group, GroupError> {
        Ok(find_group(&self.db, id).await?.into())
    }

    /// Every group, ordered by name.
    pub async fn list(&self) -> Result<Vec<Group>, GroupError> {
        let groups = group::Entity::find()
            .order_by_asc(group::Column::Name)
            .all(&self.db)
            .await?;
        Ok(groups.into_iter().map(Group::from).collect())
    }

    pub async fn update(&self, id: i64, update: GroupUpdate) -> Result<Group, GroupError> {
        let name = update.name.as_deref().map(validate_name).transpose()?;
        let note = update.note.as_deref().map(validate_note).transpose()?;
        let tx = begin_write(&self.db).await?;
        let current = writable_group(&tx, id).await?;
        let mut changes: group::ActiveModel = current.into();
        if let Some(name) = name {
            changes.name = Set(name);
        }
        if let Some(note) = note {
            changes.note = Set(note);
        }
        changes.updated_at = Set(time::now());
        let updated = changes.update(&tx).await.map_err(name_conflict)?;
        tx.commit().await?;
        Ok(updated.into())
    }

    /// Deletes the group, its memberships (cascade), the address book shares naming it and
    /// its strategy assignment (same transaction: neither has a foreign key to the group).
    pub async fn delete(&self, id: i64) -> Result<(), GroupError> {
        self.detached(move |svc| async move {
            let tx = begin_write(&svc.db).await?;
            let current = writable_group(&tx, id).await?;
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
        self.detached(move |svc| async move {
            let tx = begin_write(&svc.db).await?;
            writable_group(&tx, group_id).await?;
            find_user(&tx, user_id).await?;
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
            tx.commit().await?;
            if matches!(inserted, TryInsertResult::Inserted(n) if n > 0) {
                svc.committed(GroupChange::MembershipChanged { group_id, user_id });
            }
            Ok(())
        })
        .await
    }

    /// Removes `user_id` from the group; not a member is not an error (and reaches no hook).
    pub async fn remove_member(&self, group_id: i64, user_id: i64) -> Result<(), GroupError> {
        self.detached(move |svc| async move {
            let tx = begin_write(&svc.db).await?;
            writable_group(&tx, group_id).await?;
            let removed = group_member::Entity::delete_by_id((group_id, user_id))
                .exec(&tx)
                .await?;
            tx.commit().await?;
            if removed.rows_affected > 0 {
                svc.committed(GroupChange::MembershipChanged { group_id, user_id });
            }
            Ok(())
        })
        .await
    }

    /// Runs `op` to completion in its own task: dropping the caller's future can then
    /// neither interrupt a commit nor lose the hooks that follow it.
    async fn detached<T, F, Fut>(&self, op: F) -> Result<T, GroupError>
    where
        F: FnOnce(GroupService) -> Fut,
        Fut: Future<Output = Result<T, GroupError>> + Send + 'static,
        T: Send + 'static,
    {
        tokio::spawn(op(self.clone()))
            .await
            .map_err(|_| GroupError::Aborted)?
    }

    /// Members of the group, ordered by id.
    pub async fn members(&self, group_id: i64) -> Result<Vec<User>, GroupError> {
        let group = find_group(&self.db, group_id).await?;
        let users = group
            .find_related(user::Entity)
            .order_by_asc(user::Column::Id)
            .all(&self.db)
            .await?;
        Ok(users.into_iter().map(User::from).collect())
    }

    /// Groups the user belongs to, ordered by name.
    pub async fn groups_of_user(&self, user_id: i64) -> Result<Vec<Group>, GroupError> {
        let user = find_user(&self.db, user_id).await?;
        let groups = user
            .find_related(group::Entity)
            .order_by_asc(group::Column::Name)
            .all(&self.db)
            .await?;
        Ok(groups.into_iter().map(Group::from).collect())
    }
}

/// Ids of the groups `user_id` belongs to (address book share resolution).
pub(crate) async fn member_group_ids(
    db: &impl ConnectionTrait,
    user_id: i64,
) -> Result<Vec<i64>, DbErr> {
    group_member::Entity::find()
        .select_only()
        .column(group_member::Column::GroupId)
        .filter(group_member::Column::UserId.eq(user_id))
        .into_tuple::<i64>()
        .all(db)
        .await
}

async fn find_group(db: &impl ConnectionTrait, id: i64) -> Result<group::Model, GroupError> {
    let found = group::Entity::find_by_id(id).one(db).await?;
    found.ok_or(GroupError::NotFound)
}

async fn find_user(db: &impl ConnectionTrait, id: i64) -> Result<user::Model, GroupError> {
    let found = user::Entity::find_by_id(id).one(db).await?;
    found.ok_or(GroupError::UserNotFound)
}

/// The group, locked for the rest of the transaction, if the console may change it.
async fn writable_group(db: &impl ConnectionTrait, id: i64) -> Result<group::Model, GroupError> {
    let found = group::Entity::find_by_id(id)
        .lock_exclusive()
        .one(db)
        .await?;
    let group = found.ok_or(GroupError::NotFound)?;
    if group.managed_by == ManagedBy::Config {
        return Err(GroupError::ManagedByConfig);
    }
    Ok(group)
}

fn invalid(field: &'static str, reason: impl Into<String>) -> GroupError {
    GroupError::Invalid {
        field,
        reason: reason.into(),
    }
}

/// Trimmed name: 1-64 characters, no control characters.
fn validate_name(raw: &str) -> Result<String, GroupError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(invalid("name", "must not be blank"));
    }
    match text::problem(name, NAME_MAX_LEN, false) {
        Some(reason) => Err(invalid("name", reason)),
        None => Ok(name.to_owned()),
    }
}

fn validate_note(raw: &str) -> Result<String, GroupError> {
    match text::problem(raw, NOTE_MAX_LEN, true) {
        Some(reason) => Err(invalid("note", reason)),
        None => Ok(raw.to_owned()),
    }
}

fn name_conflict(err: DbErr) -> GroupError {
    match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => GroupError::Conflict { field: "name" },
        _ => GroupError::Database(err),
    }
}

/// A row referenced by a new membership vanished concurrently (PostgreSQL; SQLite's write
/// transaction excludes that).
fn missing_reference(err: DbErr) -> GroupError {
    if !is_foreign_key_violation(&err) {
        return GroupError::Database(err);
    }
    match err.to_string().contains(FK_MEMBER_GROUP) {
        true => GroupError::NotFound,
        false => GroupError::UserNotFound,
    }
}
