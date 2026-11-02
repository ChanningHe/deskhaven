//! Account writes: create, password (administrative reset and self-service change), delete;
//! profile, role and status go through [`UserService::edit`] (`admin`). Each `*_guarded`
//! variant runs a [`TxGuard`] inside its write transaction (authorization against the
//! locked row, plus the console audit row); the plain variants are the same writes without
//! one.

use deskhaven_db::entity::user;
use deskhaven_db::time::TimestampMs;
use deskhaven_db::{begin_write, time};

use super::events::UserChange;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, SqlErr,
};

use super::normalize::{self, Email};
use super::{
    AccountEdit, AuthSource, Credential, CredentialStamp, Inner, MAX_PASSWORD_LEN, NewUser,
    Password, PasswordError, ProfileUpdate, Role, Status, User, UserError, UserEvent, UserService,
    check_password_policy,
};
use crate::addressbooks;
use crate::console_audit::{self, NewConsoleAudit, TxGuard};
use crate::sessions::revoke_user_sessions;
use crate::strategies;

/// A [`NewUser`] whose fields passed validation (the password is checked, not yet hashed).
pub(super) struct ValidNew {
    pub new: NewUser,
    pub username_norm: String,
    pub email: Option<Email>,
    pub display_name: String,
    pub note: String,
}

impl UserService {
    /// Creates an account. Local accounts need a password that passes the policy.
    pub async fn create(&self, new: NewUser) -> Result<User, UserError> {
        self.create_guarded(new, TxGuard::none()).await
    }

    /// [`create`](Self::create), with `guard` deciding on the new account (no `before`).
    pub async fn create_guarded(
        &self,
        new: NewUser,
        guard: TxGuard<User>,
    ) -> Result<User, UserError> {
        let valid = self.inner.validate_new(new)?;
        let hash = self.inner.hash_valid(&valid).await?;
        self.detached(move |inner| async move {
            let model = inner.new_row(valid, hash);
            let tx = begin_write(&inner.db).await?;
            let created: User = model.insert(&tx).await.map_err(conflict)?.into();
            guard.check(&tx, None, Some(&created)).await?;
            tx.commit().await?;
            Ok(created)
        })
        .await
    }

    /// Validates `new` and hashes its local password (before any write lock is taken),
    /// for [`insert_prepared`](Self::insert_prepared) inside the caller's transaction.
    pub(crate) async fn prepare(&self, new: &NewUser) -> Result<PreparedUser, UserError> {
        let valid = self.inner.validate_new(new.clone())?;
        let hash = self.inner.hash_valid(&valid).await?;
        Ok(PreparedUser { valid, hash })
    }

    /// Inserts a prepared account on `conn`.
    pub(crate) async fn insert_prepared<C: ConnectionTrait>(
        &self,
        conn: &C,
        prepared: PreparedUser,
    ) -> Result<user::Model, UserError> {
        let model = self.inner.new_row(prepared.valid, prepared.hash);
        model.insert(conn).await.map_err(conflict)
    }

    /// Creates an external (LDAP / OIDC) account on `conn`, typically inside the caller's
    /// transaction, so the account and what links it to its source commit together.
    /// [`UserError::Invalid`] for a local credential (it needs a hashed password).
    pub(crate) async fn create_external<C: ConnectionTrait>(
        &self,
        conn: &C,
        new: &NewUser,
    ) -> Result<user::Model, UserError> {
        if matches!(new.credential, Credential::Local { .. }) {
            return Err(UserError::Invalid {
                field: "credential",
                reason: "an external account has no local password".to_owned(),
            });
        }
        let valid = self.inner.validate_new(new.clone())?;
        let model = self.inner.new_row(valid, None);
        model.insert(conn).await.map_err(conflict)
    }

    /// Changes profile fields (and `must_change_password`); a set `must_change_password`
    /// reaches commit hooks as [`UserChange::AccountChanged`] (cached principals drop it).
    pub async fn update_profile(&self, id: i64, update: ProfileUpdate) -> Result<User, UserError> {
        let edit = AccountEdit {
            profile: update,
            ..AccountEdit::default()
        };
        self.edit(id, edit, TxGuard::none()).await
    }

    /// Reports a committed change of the account's second factor (enabled or disabled) to
    /// the commit hooks, for [`TfaService`](crate::tfa::TfaService), which owns those
    /// columns.
    pub(crate) fn second_factor_changed(&self, user_id: i64) {
        self.inner.committed(UserChange::AccountChanged { user_id });
    }

    /// Changes the status. Leaving `active` is refused for the last active owner; entering
    /// `disabled` revokes every session of the user in the same transaction and emits
    /// [`UserEvent::Disabled`]; other transitions reach commit hooks only. Setting the
    /// current status is a no-op.
    pub async fn set_status(&self, id: i64, status: Status) -> Result<User, UserError> {
        let edit = AccountEdit {
            status: Some(status),
            ..AccountEdit::default()
        };
        self.edit(id, edit, TxGuard::none()).await
    }

    /// Changes the role; demoting the last active owner is refused. Reaches commit hooks
    /// (no event).
    pub async fn set_role(&self, id: i64, role: Role) -> Result<User, UserError> {
        let edit = AccountEdit {
            role: Some(role),
            ..AccountEdit::default()
        };
        self.edit(id, edit, TxGuard::none()).await
    }

    /// Replaces the local password (policy-checked), sets `must_change_password` and moves
    /// `password_changed_at` forward, so credential stamps taken before no longer match.
    /// Revokes every session of the user in the same transaction (durable even if the
    /// event is lost). Emits [`UserEvent::PasswordChanged`].
    pub async fn set_password(
        &self,
        id: i64,
        password: &Password,
        must_change: bool,
    ) -> Result<(), UserError> {
        self.set_password_guarded(id, password, must_change, TxGuard::none())
            .await
    }

    /// [`set_password`](Self::set_password), with `guard` deciding on the account.
    pub async fn set_password_guarded(
        &self,
        id: i64,
        password: &Password,
        must_change: bool,
        guard: TxGuard<User>,
    ) -> Result<(), UserError> {
        let target = user::Entity::find_by_id(id).one(&self.inner.db).await?;
        let target = target.ok_or(UserError::NotFound)?;
        if target.auth_source != AuthSource::Local {
            return Err(UserError::NotLocal);
        }
        // Hashed before any write lock is taken: it takes tens of milliseconds.
        let hash = self.inner.hash_new_password(password).await?;
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            // Deleted (or no longer local) since it was read: as if it never existed.
            let current = lock_user(&tx, id).await?;
            if current.auth_source != AuthSource::Local {
                return Err(UserError::NotFound);
            }
            let now = inner.now();
            let mut next = current.clone();
            next.password_hash = Some(hash);
            next.password_changed_at = Some(after(current.password_changed_at, now));
            next.must_change_password = must_change;
            next.updated_at = now;
            let (before, after_view) = (User::from(current), User::from(next.clone()));
            guard.check(&tx, Some(&before), Some(&after_view)).await?;
            next.into_active_model().reset_all().update(&tx).await?;
            revoke_user_sessions(&tx, id, now).await?;
            tx.commit().await?;
            inner.committed(UserChange::Event(UserEvent::PasswordChanged {
                user_id: id,
            }));
            Ok(())
        })
        .await
    }

    /// Re-authentication before a sensitive self-service change (console TOTP enrolment):
    /// verifies `password` against the local account `id`, one Argon2id computation either
    /// way. [`UserError::WrongPassword`] on mismatch, [`UserError::NotLocal`] for an account
    /// without a local password.
    pub async fn verify_password(&self, id: i64, password: &Password) -> Result<(), UserError> {
        self.verified_local(id, password).await.map(drop)
    }

    /// The local account `id` and its stored hash, once `password` matched it.
    async fn verified_local(
        &self,
        id: i64,
        password: &Password,
    ) -> Result<(user::Model, String), UserError> {
        let target = user::Entity::find_by_id(id).one(&self.inner.db).await?;
        let target = target.ok_or(UserError::NotFound)?;
        let stored = match (&target.auth_source, &target.password_hash) {
            (AuthSource::Local, Some(hash)) => hash.clone(),
            _ => return Err(UserError::NotLocal),
        };
        if !self.inner.verify_current(id, password, &stored).await? {
            return Err(UserError::WrongPassword);
        }
        Ok((target, stored))
    }

    /// Self-service password change (console): verifies `current` against the stored hash
    /// (one Argon2id computation either way), then, in one transaction, replaces it with
    /// `new` (policy-checked; `must_change_password` cleared), revokes every session of the
    /// user and records `audit`. Returns the new password's credential stamp, for the
    /// caller's fresh session. [`UserError::WrongPassword`] when `current` does not match,
    /// also when the password changed concurrently. Emits [`UserEvent::PasswordChanged`].
    pub async fn change_password(
        &self,
        id: i64,
        current: &Password,
        new: &Password,
        audit: &NewConsoleAudit,
    ) -> Result<CredentialStamp, UserError> {
        let (target, stored) = self.verified_local(id, current).await?;
        let hash = self.inner.hash_new_password(new).await?;
        let change = PasswordChange {
            id,
            hash,
            previous: target.password_changed_at,
            replaces: stored,
            audit: audit.clone(),
        };
        let changed_at = self
            .detached(move |inner| async move { inner.replace_password(change).await })
            .await?
            .ok_or(UserError::WrongPassword)?;
        Ok(CredentialStamp::from_parts(
            Some(changed_at.timestamp_millis()),
            AuthSource::Local,
        ))
    }

    /// Deletes the account; memberships, sessions, identities and tokens go with it (FK
    /// cascades), as do the personal address book with its peers and the address book
    /// shares naming the user (`addressbooks::forget_user`, same transaction). Shared books
    /// the user owned stay, unowned. Refused for the last active owner. Emits
    /// [`UserEvent::Deleted`].
    pub async fn delete(&self, id: i64) -> Result<(), UserError> {
        self.delete_guarded(id, TxGuard::none()).await
    }

    /// [`delete`](Self::delete), with `guard` deciding on the account (no `after`).
    pub async fn delete_guarded(&self, id: i64, guard: TxGuard<User>) -> Result<(), UserError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let owners = lock_active_owners(&tx).await?;
            let current = lock_user(&tx, id).await?;
            guard
                .check(&tx, Some(&current.clone().into()), None)
                .await?;
            ensure_not_last_owner(&current, &owners)?;
            addressbooks::forget_user(&tx, id).await?;
            strategies::forget_user(&tx, id).await?;
            user::Entity::delete_by_id(id).exec(&tx).await?;
            tx.commit().await?;
            inner.committed(UserChange::Event(UserEvent::Deleted { user_id: id }));
            Ok(())
        })
        .await
    }
}

/// A validated account, its local password hashed.
pub(crate) struct PreparedUser {
    valid: ValidNew,
    hash: Option<String>,
}

/// A self-service password change, ready to write.
struct PasswordChange {
    id: i64,
    hash: String,
    previous: Option<TimestampMs>,
    /// Only replace this stored hash (the one just verified).
    replaces: String,
    audit: NewConsoleAudit,
}

impl Inner {
    /// Policy check with the current minimum length, then a fresh hash.
    async fn hash_new_password(&self, password: &Password) -> Result<String, UserError> {
        check_password_policy(password, self.min_password_length())
            .map_err(|reason| UserError::PolicyViolation { reason })?;
        Ok(self.hasher.hash(password).await?)
    }

    /// Whether `password` matches `stored` (an over-long one is refused after the dummy
    /// cost, a malformed hash never matches).
    async fn verify_current(
        &self,
        id: i64,
        password: &Password,
        stored: &str,
    ) -> Result<bool, UserError> {
        if password.char_len() > MAX_PASSWORD_LEN {
            self.hasher.verify_dummy(&Password::new("")).await?;
            return Ok(false);
        }
        match self.hasher.verify(password, stored).await {
            Ok(matched) => Ok(matched),
            Err(PasswordError::MalformedHash) => {
                tracing::error!(user_id = id, "stored password hash is malformed");
                self.hasher.verify_dummy(password).await?;
                Ok(false)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Writes the new hash (clearing `must_change_password`), revokes every session of the
    /// user and records the audit row, in one transaction. The new `password_changed_at`,
    /// or `None` when no row matched (deleted, no longer local, or the hash to replace is
    /// gone).
    async fn replace_password(
        &self,
        change: PasswordChange,
    ) -> Result<Option<TimestampMs>, UserError> {
        let now = self.now();
        let changed_at = after(change.previous, now);
        let changes = user::ActiveModel {
            password_hash: Set(Some(change.hash)),
            password_changed_at: Set(Some(changed_at)),
            must_change_password: Set(false),
            updated_at: Set(now),
            ..Default::default()
        };
        let tx = begin_write(&self.db).await?;
        let updated = user::Entity::update_many()
            .set(changes)
            .filter(user::Column::Id.eq(change.id))
            .filter(user::Column::AuthSource.eq(AuthSource::Local))
            .filter(user::Column::PasswordHash.eq(change.replaces))
            .exec(&tx)
            .await?;
        if updated.rows_affected == 0 {
            return Ok(None);
        }
        revoke_user_sessions(&tx, change.id, now).await?;
        console_audit::insert(&tx, &change.audit, now.timestamp_millis()).await?;
        tx.commit().await?;
        self.committed(UserChange::Event(UserEvent::PasswordChanged {
            user_id: change.id,
        }));
        Ok(Some(changed_at))
    }

    /// Every field of `new` checked and normalized; a local password against the policy.
    pub(super) fn validate_new(&self, new: NewUser) -> Result<ValidNew, UserError> {
        let username_norm = normalize::username(&new.username)?;
        let email = new.email.as_deref().map(normalize::email).transpose()?;
        let display_name = normalize::display_name(&new.display_name, &new.username)?;
        let note = normalize::note(&new.note)?;
        if let Credential::Local { password } = &new.credential {
            check_password_policy(password, self.min_password_length())
                .map_err(|reason| UserError::PolicyViolation { reason })?;
        }
        Ok(ValidNew {
            new,
            username_norm,
            email,
            display_name,
            note,
        })
    }

    /// The hash of a validated local password; `None` for directory accounts.
    pub(super) async fn hash_valid(&self, valid: &ValidNew) -> Result<Option<String>, UserError> {
        match &valid.new.credential {
            Credential::Local { password } => Ok(Some(self.hasher.hash(password).await?)),
            Credential::Ldap | Credential::Oidc => Ok(None),
        }
    }

    pub(super) fn new_row(
        &self,
        valid: ValidNew,
        password_hash: Option<String>,
    ) -> user::ActiveModel {
        let ValidNew {
            new,
            username_norm,
            email,
            display_name,
            note,
        } = valid;
        let (email, email_norm) = split_email(email);
        let now = self.now();
        user::ActiveModel {
            username: Set(new.username),
            username_norm: Set(username_norm),
            display_name: Set(display_name),
            email: Set(email),
            email_norm: Set(email_norm),
            email_verified: Set(new.email_verified),
            password_changed_at: Set(password_hash.as_ref().map(|_| now)),
            password_hash: Set(password_hash),
            auth_source: Set(new.credential.auth_source()),
            role: Set(new.role),
            status: Set(new.status),
            must_change_password: Set(new.must_change_password),
            note: Set(note),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        }
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

pub(super) fn split_email(email: Option<Email>) -> (Option<String>, Option<String>) {
    match email {
        Some(Email { value, norm }) => (Some(value), Some(norm)),
        None => (None, None),
    }
}

/// The user row, locked for the rest of the transaction (PostgreSQL; SQLite's write
/// transaction already excludes other writers).
pub(super) async fn lock_user(tx: &DatabaseTransaction, id: i64) -> Result<user::Model, UserError> {
    let found = user::Entity::find_by_id(id)
        .lock_exclusive()
        .one(tx)
        .await?;
    found.ok_or(UserError::NotFound)
}

/// Ids of all active owners, locked in id order (a consistent order, so concurrent
/// owner-affecting transactions queue instead of deadlocking).
pub(super) async fn lock_active_owners(tx: &DatabaseTransaction) -> Result<Vec<i64>, DbErr> {
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
pub(super) fn ensure_not_last_owner(
    user: &user::Model,
    active_owners: &[i64],
) -> Result<(), UserError> {
    let is_active_owner = user.role == Role::Owner && user.status == Status::Active;
    let has_other = active_owners.iter().any(|&owner| owner != user.id);
    if is_active_owner && !has_other {
        return Err(UserError::LastOwner);
    }
    Ok(())
}

/// Maps unique-index violations on the normalized columns to [`UserError::Conflict`].
/// Both backends name the column (SQLite) or the index (PostgreSQL) in the message.
pub(super) fn conflict(err: DbErr) -> UserError {
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
