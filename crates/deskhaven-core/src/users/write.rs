//! Account writes: create, profile, password, status, role, delete.

use deskhaven_db::entity::user;
use deskhaven_db::time::TimestampMs;
use deskhaven_db::{begin_write, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, SqlErr,
};

use super::normalize::{self, Email};
use super::{
    AuthSource, Credential, Inner, NewUser, Password, ProfileUpdate, Role, Status, User, UserError,
    UserEvent, UserService, check_password_policy,
};

impl UserService {
    /// Creates an account. Local accounts need a password that passes the policy.
    pub async fn create(&self, new: NewUser) -> Result<User, UserError> {
        let username_norm = normalize::username(&new.username)?;
        let email = new.email.as_deref().map(normalize::email).transpose()?;
        let display_name = normalize::display_name(&new.display_name, &new.username)?;
        let note = normalize::note(&new.note)?;
        let password_hash = match &new.credential {
            Credential::Local { password } => Some(self.inner.hash_new_password(password).await?),
            Credential::Ldap | Credential::Oidc => None,
        };
        let (email, email_norm) = split_email(email);
        let now = time::now();
        let password_changed_at = password_hash.as_ref().map(|_| now);
        let model = user::ActiveModel {
            username: Set(new.username),
            username_norm: Set(username_norm),
            display_name: Set(display_name),
            email: Set(email),
            email_norm: Set(email_norm),
            email_verified: Set(new.email_verified),
            password_hash: Set(password_hash),
            password_changed_at: Set(password_changed_at),
            auth_source: Set(new.credential.auth_source()),
            role: Set(new.role),
            status: Set(new.status),
            must_change_password: Set(new.must_change_password),
            note: Set(note),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        let created = model.insert(&self.inner.db).await.map_err(conflict)?;
        Ok(created.into())
    }

    pub async fn update_profile(&self, id: i64, update: ProfileUpdate) -> Result<User, UserError> {
        let email = match &update.email {
            Some(email) => Some(email.as_deref().map(normalize::email).transpose()?),
            None => None,
        };
        let note = update.note.as_deref().map(normalize::note).transpose()?;
        let tx = begin_write(&self.inner.db).await?;
        let current = lock_user(&tx, id).await?;
        let mut changes: user::ActiveModel = current.clone().into();
        if let Some(name) = &update.display_name {
            changes.display_name = Set(normalize::display_name(name, &current.username)?);
        }
        if let Some(email) = email {
            let moved = email.as_ref().map(|e| e.norm.as_str()) != current.email_norm.as_deref();
            if moved {
                changes.email_verified = Set(false);
            }
            let (email, email_norm) = split_email(email);
            changes.email = Set(email);
            changes.email_norm = Set(email_norm);
        }
        if let Some(verified) = update.email_verified {
            changes.email_verified = Set(verified);
        }
        if let Some(must_change) = update.must_change_password {
            changes.must_change_password = Set(must_change);
        }
        if let Some(note) = note {
            changes.note = Set(note);
        }
        changes.updated_at = Set(time::now());
        let updated = changes.update(&tx).await.map_err(conflict)?;
        tx.commit().await?;
        Ok(updated.into())
    }

    /// Replaces the local password (policy-checked), sets `must_change_password` and moves
    /// `password_changed_at` forward, so credential stamps taken before no longer match.
    /// Emits [`UserEvent::PasswordChanged`].
    pub async fn set_password(
        &self,
        id: i64,
        password: &Password,
        must_change: bool,
    ) -> Result<(), UserError> {
        let target = user::Entity::find_by_id(id).one(&self.inner.db).await?;
        let target = target.ok_or(UserError::NotFound)?;
        if target.auth_source != AuthSource::Local {
            return Err(UserError::NotLocal);
        }
        // Hashed before any write lock is taken: it takes tens of milliseconds.
        let hash = self.inner.hash_new_password(password).await?;
        let previous = target.password_changed_at;
        self.detached(move |inner| async move {
            let now = time::now();
            let changes = user::ActiveModel {
                password_hash: Set(Some(hash)),
                password_changed_at: Set(Some(after(previous, now))),
                must_change_password: Set(must_change),
                updated_at: Set(now),
                ..Default::default()
            };
            let result = user::Entity::update_many()
                .set(changes)
                .filter(user::Column::Id.eq(id))
                .filter(user::Column::AuthSource.eq(AuthSource::Local))
                .exec(&inner.db)
                .await?;
            // Deleted (or no longer local) since it was read.
            if result.rows_affected == 0 {
                return Err(UserError::NotFound);
            }
            inner.emit(UserEvent::PasswordChanged { user_id: id });
            Ok(())
        })
        .await
    }

    /// Changes the status. Leaving `active` is refused for the last active owner; entering
    /// `disabled` emits [`UserEvent::Disabled`]. Setting the current status is a no-op.
    pub async fn set_status(&self, id: i64, status: Status) -> Result<User, UserError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let owners = lock_active_owners(&tx).await?;
            let current = lock_user(&tx, id).await?;
            if current.status == status {
                return Ok(current.into());
            }
            if status != Status::Active {
                ensure_not_last_owner(&current, &owners)?;
            }
            let mut changes: user::ActiveModel = current.into();
            changes.status = Set(status);
            changes.updated_at = Set(time::now());
            let updated = changes.update(&tx).await?;
            tx.commit().await?;
            if status == Status::Disabled {
                inner.emit(UserEvent::Disabled { user_id: id });
            }
            Ok(updated.into())
        })
        .await
    }

    /// Changes the role; demoting the last active owner is refused.
    pub async fn set_role(&self, id: i64, role: Role) -> Result<User, UserError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let owners = lock_active_owners(&tx).await?;
            let current = lock_user(&tx, id).await?;
            if current.role == role {
                return Ok(current.into());
            }
            if role != Role::Owner {
                ensure_not_last_owner(&current, &owners)?;
            }
            let mut changes: user::ActiveModel = current.into();
            changes.role = Set(role);
            changes.updated_at = Set(time::now());
            let updated = changes.update(&tx).await?;
            tx.commit().await?;
            Ok(updated.into())
        })
        .await
    }

    /// Deletes the account; memberships, sessions, identities and tokens go with it (FK
    /// cascades). Refused for the last active owner. Emits [`UserEvent::Deleted`].
    pub async fn delete(&self, id: i64) -> Result<(), UserError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let owners = lock_active_owners(&tx).await?;
            let current = lock_user(&tx, id).await?;
            ensure_not_last_owner(&current, &owners)?;
            user::Entity::delete_by_id(id).exec(&tx).await?;
            tx.commit().await?;
            inner.emit(UserEvent::Deleted { user_id: id });
            Ok(())
        })
        .await
    }
}

impl Inner {
    /// Policy check with the current minimum length, then a fresh hash.
    async fn hash_new_password(&self, password: &Password) -> Result<String, UserError> {
        check_password_policy(password, self.min_password_length())
            .map_err(|reason| UserError::PolicyViolation { reason })?;
        Ok(self.hasher.hash(password).await?)
    }
}

/// `now`, or one millisecond past `previous` when the clock has not moved beyond it, so a
/// password change always produces a new credential stamp. Known residue: two changes
/// racing within one millisecond (both reading the same `previous`) can share a stamp.
fn after(previous: Option<TimestampMs>, now: TimestampMs) -> TimestampMs {
    let next = previous.and_then(|p| time::from_millis(p.timestamp_millis().saturating_add(1)));
    match next {
        Some(next) if next.timestamp_millis() > now.timestamp_millis() => next,
        _ => now,
    }
}

fn split_email(email: Option<Email>) -> (Option<String>, Option<String>) {
    match email {
        Some(Email { value, norm }) => (Some(value), Some(norm)),
        None => (None, None),
    }
}

/// The user row, locked for the rest of the transaction (PostgreSQL; SQLite's write
/// transaction already excludes other writers).
async fn lock_user(tx: &DatabaseTransaction, id: i64) -> Result<user::Model, UserError> {
    let found = user::Entity::find_by_id(id)
        .lock_exclusive()
        .one(tx)
        .await?;
    found.ok_or(UserError::NotFound)
}

/// Ids of all active owners, locked in id order (a consistent order, so concurrent
/// owner-affecting transactions queue instead of deadlocking).
async fn lock_active_owners(tx: &DatabaseTransaction) -> Result<Vec<i64>, DbErr> {
    user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .filter(user::Column::Role.eq(Role::Owner))
        .filter(user::Column::Status.eq(Status::Active))
        .order_by_asc(user::Column::Id)
        .lock_exclusive()
        .into_tuple::<i64>()
        .all(tx)
        .await
}

/// Refuses to take `user` out of the active owners when no other active owner remains.
fn ensure_not_last_owner(user: &user::Model, active_owners: &[i64]) -> Result<(), UserError> {
    let is_active_owner = user.role == Role::Owner && user.status == Status::Active;
    let has_other = active_owners.iter().any(|&owner| owner != user.id);
    if is_active_owner && !has_other {
        return Err(UserError::LastOwner);
    }
    Ok(())
}

/// Maps unique-index violations on the normalized columns to [`UserError::Conflict`].
/// Both backends name the column (SQLite) or the index (PostgreSQL) in the message.
fn conflict(err: DbErr) -> UserError {
    let field = match err.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(msg)) if msg.contains("email_norm") => Some("email"),
        Some(SqlErr::UniqueConstraintViolation(msg)) if msg.contains("username_norm") => {
            Some("username")
        }
        _ => None,
    };
    match field {
        Some(field) => UserError::Conflict { field },
        None => UserError::Database(err),
    }
}
