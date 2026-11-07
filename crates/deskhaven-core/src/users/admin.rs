//! Administrative account changes: one combined edit (profile, role, status), batch
//! creation for imports, and temporary passwords.

use std::collections::HashMap;

use deskhaven_db::begin_write;
use deskhaven_db::entity::user;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, IntoActiveModel, QueryFilter,
    QuerySelect,
};
use tokio::task::JoinSet;
use zeroize::Zeroizing;

use super::events::UserChange;
use super::normalize::{self, Email};
use super::write::{
    ValidNew, conflict, ensure_not_last_owner, lock_active_owners, lock_user, split_email,
};
use super::{
    Credential, Password, ProfileUpdate, Role, Status, User, UserError, UserEvent, UserService,
};
use crate::console_audit::{GuardError, TxGuard};
use crate::crypto::fill_random;

/// Most accounts [`UserService::create_batch`] takes at once.
pub const MAX_BATCH: usize = 500;
/// Length of a temporary password when the policy minimum is shorter.
pub const TEMPORARY_PASSWORD_LEN: usize = 20;
/// 64 symbols, so `byte % 64` is uniform: 6 bits per character.
const TEMPORARY_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Changes to one account, applied in one transaction; `None` leaves a part as it is.
#[derive(Clone, Debug, Default)]
pub struct AccountEdit {
    pub profile: ProfileUpdate,
    pub role: Option<Role>,
    pub status: Option<Status>,
}

/// A row of a batch that cannot be created, by its position in the batch.
#[derive(Debug)]
pub struct RowError {
    pub index: usize,
    pub error: UserError,
}

/// Why [`UserService::create_batch`] created nothing.
#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    /// Rows that are invalid, duplicate each other or existing accounts, or were refused.
    #[error("{} row(s) rejected", .0.len())]
    Rows(Vec<RowError>),
    #[error(transparent)]
    Failed(#[from] UserError),
}

/// [`ProfileUpdate`] with its email and note validated.
struct ValidProfile {
    update: ProfileUpdate,
    email: Option<Option<Email>>,
    note: Option<String>,
}

impl ValidProfile {
    fn parse(update: ProfileUpdate) -> Result<Self, UserError> {
        let email = match &update.email {
            Some(email) => Some(email.as_deref().map(normalize::email).transpose()?),
            None => None,
        };
        let note = update.note.as_deref().map(normalize::note).transpose()?;
        Ok(Self {
            update,
            email,
            note,
        })
    }

    fn is_empty(&self) -> bool {
        let u = &self.update;
        u.display_name.is_none()
            && u.email.is_none()
            && u.email_verified.is_none()
            && u.must_change_password.is_none()
            && u.note.is_none()
    }

    /// Applies the profile fields to `row`.
    fn apply(self, row: &mut user::Model) -> Result<(), UserError> {
        if let Some(name) = &self.update.display_name {
            row.display_name = normalize::display_name(name, &row.username)?;
        }
        if let Some(email) = self.email {
            let moved = email.as_ref().map(|e| e.norm.as_str()) != row.email_norm.as_deref();
            if moved {
                row.email_verified = false;
            }
            (row.email, row.email_norm) = split_email(email);
        }
        if let Some(verified) = self.update.email_verified {
            if verified && row.email.is_none() {
                return Err(UserError::Invalid {
                    field: "email_verified",
                    reason: "the account has no email address".to_owned(),
                });
            }
            row.email_verified = verified;
        }
        if let Some(must_change) = self.update.must_change_password {
            row.must_change_password = must_change;
        }
        if let Some(note) = self.note {
            row.note = note;
        }
        Ok(())
    }
}

impl UserService {
    /// Applies `edit` in one transaction. A different email (case-insensitively) resets
    /// `email_verified` unless set explicitly. Demoting or deactivating the last active
    /// owner is refused; entering `disabled` revokes every session in the same transaction
    /// and emits [`UserEvent::Disabled`]; entering `disabled` or the `user` role revokes the
    /// account's API tokens in the same transaction; role and other status changes reach
    /// commit hooks, as does a set `must_change_password` ([`UserChange::AccountChanged`]).
    /// `guard` sees the account before and after (a no-op edit included).
    pub async fn edit(
        &self,
        id: i64,
        edit: AccountEdit,
        guard: TxGuard<User>,
    ) -> Result<User, UserError> {
        let profile = ValidProfile::parse(edit.profile)?;
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let owners = lock_active_owners(&tx).await?;
            let current = lock_user(&tx, id).await?;
            let role = edit.role.filter(|role| *role != current.role);
            let status = edit.status.filter(|status| *status != current.status);
            let touched = !profile.is_empty() || role.is_some() || status.is_some();
            // The console's account-setup gate reads this from the cached principal.
            let must_change_set = profile.update.must_change_password.is_some();
            let mut next = current.clone();
            profile.apply(&mut next)?;
            next.role = role.unwrap_or(next.role);
            next.status = status.unwrap_or(next.status);
            let now = inner.now();
            if touched {
                next.updated_at = now;
            }
            let (before, after) = (User::from(current.clone()), User::from(next.clone()));
            guard.check(&tx, Some(&before), Some(&after)).await?;
            let demoted = role.is_some_and(|role| role != Role::Owner);
            if demoted || status.is_some_and(|status| status != Status::Active) {
                ensure_not_last_owner(&current, &owners)?;
            }
            if touched {
                let row = next.into_active_model().reset_all();
                row.update(&tx).await.map_err(conflict)?;
            }
            if status == Some(Status::Disabled) {
                // Durable: re-enabling the account must not resurrect old sessions.
                crate::sessions::revoke_user_sessions(&tx, id, now).await?;
            }
            // Likewise its API tokens, which carry its (administrator) authority.
            if status == Some(Status::Disabled) || role == Some(Role::User) {
                crate::api_tokens::revoke_created_by(&tx, id, now).await?;
            }
            tx.commit().await?;
            if must_change_set {
                inner.committed(UserChange::AccountChanged { user_id: id });
            }
            if role.is_some() {
                inner.committed(UserChange::RoleChanged { user_id: id });
            }
            match status {
                Some(Status::Disabled) => {
                    inner.committed(UserChange::Event(UserEvent::Disabled { user_id: id }));
                }
                Some(_) => inner.committed(UserChange::StatusChanged { user_id: id }),
                None => {}
            }
            Ok(after)
        })
        .await
    }

    /// Creates every account of `rows` in one transaction, or none. Every row is validated
    /// first and every problem reported at once ([`BatchError::Rows`]): invalid fields, a
    /// username or email repeated in the batch or already taken, and (inside the
    /// transaction) a guard's refusal. At most [`MAX_BATCH`] rows.
    pub async fn create_batch(
        &self,
        rows: Vec<(super::NewUser, TxGuard<User>)>,
    ) -> Result<Vec<User>, BatchError> {
        if rows.len() > MAX_BATCH {
            return Err(UserError::Invalid {
                field: "rows",
                reason: format!("at most {MAX_BATCH} accounts at once"),
            }
            .into());
        }
        let (news, guards): (Vec<_>, Vec<_>) = rows.into_iter().unzip();
        let (valid, errors) = self.validate_batch(news).await?;
        if !errors.is_empty() {
            return Err(BatchError::Rows(errors));
        }
        let hashes = self.hash_batch(&valid).await?;
        let rows: Vec<_> = valid.into_iter().zip(hashes).zip(guards).collect();
        let inner = std::sync::Arc::clone(&self.inner);
        let task = async move {
            let tx = begin_write(&inner.db).await.map_err(UserError::from)?;
            let mut created = Vec::with_capacity(rows.len());
            for (index, ((valid, hash), guard)) in rows.into_iter().enumerate() {
                // A row's own problem (taken meanwhile, refused) rejects the batch with that
                // row; anything else (database, audit insert) is a failure of the request.
                let row_error = |error| BatchError::Rows(vec![RowError { index, error }]);
                let user: User = match inner.new_row(valid, hash).insert(&tx).await {
                    Ok(model) => model.into(),
                    Err(err) => match conflict(err) {
                        conflict @ UserError::Conflict { .. } => return Err(row_error(conflict)),
                        other => return Err(BatchError::Failed(other)),
                    },
                };
                match guard.check(&tx, None, Some(&user)).await {
                    Ok(()) => {}
                    Err(GuardError::Refused(refused)) => {
                        return Err(row_error(UserError::Refused(refused)));
                    }
                    Err(GuardError::Database(db)) => return Err(BatchError::Failed(db.into())),
                }
                created.push(user);
            }
            tx.commit().await.map_err(UserError::from)?;
            Ok(created)
        };
        tokio::spawn(task)
            .await
            .map_err(|_| BatchError::Failed(UserError::Aborted))?
    }

    /// The hashes of `rows`' local passwords, in order (`None` for directory accounts),
    /// computed concurrently: one fewer at a time than the hasher has slots, so a login
    /// never waits behind a whole import, only behind one computation.
    async fn hash_batch(&self, rows: &[ValidNew]) -> Result<Vec<Option<String>>, UserError> {
        let width = self.inner.hasher.max_concurrent().saturating_sub(1).max(1);
        let passwords = rows.iter().map(|row| match &row.new.credential {
            Credential::Local { password } => Some(password.clone()),
            Credential::Ldap | Credential::Oidc => None,
        });
        let mut pending = passwords.enumerate();
        let mut hashes = vec![None; rows.len()];
        let mut running = JoinSet::new();
        loop {
            while running.len() < width {
                let Some((index, password)) = pending.next() else {
                    break;
                };
                let Some(password) = password else { continue };
                let hasher = self.inner.hasher.clone();
                running.spawn(async move { (index, hasher.hash(&password).await) });
            }
            let Some(done) = running.join_next().await else {
                break;
            };
            let (index, hash) = done.map_err(|_| UserError::Aborted)?;
            hashes[index] = Some(hash?);
        }
        Ok(hashes)
    }

    /// What [`create_batch`](Self::create_batch) would reject in `rows` (sorted by index),
    /// without creating anything: invalid fields, duplicates within the batch, usernames
    /// or emails already taken. Empty when every row is acceptable now.
    pub async fn check_batch(&self, rows: Vec<super::NewUser>) -> Result<Vec<RowError>, UserError> {
        Ok(self.validate_batch(rows).await?.1)
    }

    /// The valid rows (in order) and the problems of all rows, sorted by index.
    async fn validate_batch(
        &self,
        rows: Vec<super::NewUser>,
    ) -> Result<(Vec<ValidNew>, Vec<RowError>), UserError> {
        let mut errors = Vec::new();
        let mut valid = Vec::with_capacity(rows.len());
        for (index, new) in rows.into_iter().enumerate() {
            match self.inner.validate_new(new) {
                Ok(row) => valid.push((index, row)),
                Err(error) => errors.push(RowError { index, error }),
            }
        }
        errors.extend(duplicates(&valid));
        errors.extend(self.taken(&valid).await?);
        errors.sort_by_key(|e| e.index);
        Ok((valid.into_iter().map(|(_, row)| row).collect(), errors))
    }

    /// Rows whose username or email an existing account already has.
    async fn taken(&self, rows: &[(usize, ValidNew)]) -> Result<Vec<RowError>, UserError> {
        let usernames: Vec<&str> = rows.iter().map(|(_, r)| r.username_norm.as_str()).collect();
        let emails: Vec<&str> = rows.iter().filter_map(|(_, r)| email_norm(r)).collect();
        let existing: Vec<(String, Option<String>)> = user::Entity::find()
            .select_only()
            .column(user::Column::UsernameNorm)
            .column(user::Column::EmailNorm)
            .filter(
                Condition::any()
                    .add(user::Column::UsernameNorm.is_in(usernames))
                    .add(user::Column::EmailNorm.is_in(emails)),
            )
            .into_tuple()
            .all(&self.inner.db)
            .await?;
        let has_username = |n: &str| existing.iter().any(|(u, _)| u == n);
        let has_email = |n: &str| existing.iter().any(|(_, e)| e.as_deref() == Some(n));
        let mut errors = Vec::new();
        for (index, row) in rows {
            let field = if has_username(&row.username_norm) {
                Some("username")
            } else {
                email_norm(row).filter(|e| has_email(e)).map(|_| "email")
            };
            if let Some(field) = field {
                errors.push(RowError {
                    index: *index,
                    error: UserError::Conflict { field },
                });
            }
        }
        Ok(errors)
    }

    /// A random password for an administrator to hand over (with `must_change_password`):
    /// [`TEMPORARY_PASSWORD_LEN`] characters or the policy minimum, if longer.
    pub fn temporary_password(&self) -> Result<Password, UserError> {
        let min = usize::try_from(self.inner.min_password_length()).unwrap_or(usize::MAX);
        let len = TEMPORARY_PASSWORD_LEN.max(min);
        let mut bytes = Zeroizing::new(vec![0u8; len]);
        fill_random(&mut bytes).map_err(|_| UserError::Random)?;
        let symbol = |b: &u8| TEMPORARY_ALPHABET[usize::from(*b) % TEMPORARY_ALPHABET.len()];
        let text: String = bytes.iter().map(|b| char::from(symbol(b))).collect();
        Ok(Password::new(text))
    }
}

fn email_norm(row: &ValidNew) -> Option<&str> {
    row.email.as_ref().map(|e| e.norm.as_str())
}

/// Second and later rows repeating a username or email of an earlier row.
fn duplicates(rows: &[(usize, ValidNew)]) -> Vec<RowError> {
    let mut seen: HashMap<(&'static str, &str), usize> = HashMap::new();
    let mut errors = Vec::new();
    for (index, row) in rows {
        let keys = [
            ("username", Some(row.username_norm.as_str())),
            ("email", email_norm(row)),
        ];
        let mut repeated = None;
        for (field, value) in keys {
            let Some(value) = value else { continue };
            if seen.insert((field, value), *index).is_some() && repeated.is_none() {
                repeated = Some(field);
            }
        }
        if let Some(field) = repeated {
            errors.push(RowError {
                index: *index,
                error: UserError::Conflict { field },
            });
        }
    }
    errors
}
