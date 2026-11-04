//! Local password login (06 §3.1, §3.6) and the credential stamp that closes the
//! login / password-change race (06 §3.2).

use deskhaven_db::entity::user;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect};

use super::{
    AuthError, AuthSource, Inner, MAX_PASSWORD_LEN, Password, PasswordError, Status, User,
    UserError, UserService,
};

/// The credential a login verified: the user's auth source and `password_changed_at` at
/// that moment. Opaque; compare it with [`UserService::credential_stamp_matches`].
///
/// Local logins take it with `auth_source = local`. An OIDC login takes it from the account
/// it resolved to, whatever its source, so a password change, a disabled account or a
/// changed auth source between the verification and the session refuses the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CredentialStamp {
    password_changed_at: Option<i64>,
    auth_source: AuthSource,
}

impl CredentialStamp {
    pub(crate) fn of(user: &user::Model) -> Self {
        Self {
            password_changed_at: user.password_changed_at.map(|t| t.timestamp_millis()),
            auth_source: user.auth_source,
        }
    }

    /// Both parts, for storing a stamp of any source server side (the 2FA challenge between
    /// the two login steps, OIDC flows); never sent to a client.
    pub(crate) fn parts(self) -> (Option<i64>, AuthSource) {
        (self.password_changed_at, self.auth_source)
    }

    pub(crate) fn from_parts(password_changed_at: Option<i64>, auth_source: AuthSource) -> Self {
        Self {
            password_changed_at,
            auth_source,
        }
    }
}

/// A successful local login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authenticated {
    pub user: User,
    /// Re-check with [`UserService::credential_stamp_matches`] inside the transaction that
    /// creates the session.
    pub stamp: CredentialStamp,
}

impl UserService {
    /// Verifies a local username / password pair.
    ///
    /// Unknown usernames, malformed usernames, over-long passwords, external (LDAP / OIDC)
    /// accounts, wrong passwords and corrupt stored hashes all fail with
    /// [`AuthError::InvalidCredentials`] after one Argon2id computation, so neither the error
    /// nor the timing tells whether the account exists (corrupt hashes are logged). A hash
    /// with outdated parameters is replaced on success, best effort: a failed rehash is
    /// logged and the login still succeeds.
    ///
    /// Only verifies the password: rate limiting, lockout, 2FA and sessions are the caller's.
    /// [`AuthError::Password`] with [`PasswordError::Busy`] means overload (APIs: 503).
    pub async fn authenticate_local(
        &self,
        username: &str,
        password: &Password,
    ) -> Result<Authenticated, AuthError> {
        let inner = &self.inner;
        if password.char_len() > MAX_PASSWORD_LEN {
            // Never hash attacker-sized input; spend the usual time on an empty one instead.
            inner.hasher.verify_dummy(&Password::new("")).await?;
            return Err(AuthError::InvalidCredentials);
        }
        let candidate = inner
            .find_by_username(username)
            .await?
            .filter(|u| u.auth_source == AuthSource::Local)
            .and_then(|u| u.password_hash.clone().map(|hash| (u, hash)));
        let Some((user, hash)) = candidate else {
            inner.hasher.verify_dummy(password).await?;
            return Err(AuthError::InvalidCredentials);
        };
        if !inner.verify_stored(user.id, password, &hash).await? {
            return Err(AuthError::InvalidCredentials);
        }
        match user.status {
            Status::Active => {}
            Status::Disabled => return Err(AuthError::Disabled),
            Status::Unverified => return Err(AuthError::Unverified),
        }
        if inner.hasher.needs_rehash(&hash)
            && let Err(err) = inner.rehash(user.id, &hash, password).await
        {
            tracing::warn!(user_id = user.id, error = ?err, "password rehash failed; keeping the old hash");
        }
        let stamp = CredentialStamp::of(&user);
        Ok(Authenticated {
            user: user.into(),
            stamp,
        })
    }

    /// The Argon2id work of one local login, spent on nothing: an external (LDAP) password
    /// check costs the same as a local one, and waits for a hasher slot like one
    /// ([`PasswordError::Busy`] when saturated). Over-long input hashes an empty password.
    pub(crate) async fn spend_dummy_hash(&self, password: &Password) -> Result<(), AuthError> {
        let hasher = &self.inner.hasher;
        if password.char_len() > MAX_PASSWORD_LEN {
            hasher.verify_dummy(&Password::new("")).await?;
        } else {
            hasher.verify_dummy(password).await?;
        }
        Ok(())
    }

    /// Whether `user_id` still exists, is active, and has the auth source and password
    /// `stamp` was taken from (a local login's stamp: still a local account with the same
    /// password).
    ///
    /// Call it on the connection of the transaction that creates the session: on PostgreSQL
    /// the user row is share-locked until that transaction ends, so a concurrent password
    /// change waits for it and its [`UserEvent`](super::UserEvent) then revokes the new
    /// session; on SQLite the write transaction already excludes other writers.
    pub async fn credential_stamp_matches<C: ConnectionTrait>(
        &self,
        conn: &C,
        user_id: i64,
        stamp: &CredentialStamp,
    ) -> Result<bool, UserError> {
        let found = user::Entity::find_by_id(user_id)
            .lock_shared()
            .one(conn)
            .await?;
        Ok(found.is_some_and(|u| u.status == Status::Active && CredentialStamp::of(&u) == *stamp))
    }
}

impl Inner {
    /// Verifies against the stored hash. A corrupt hash is logged and treated as a mismatch
    /// after the dummy cost, so it fails closed without an enumeration signal.
    async fn verify_stored(
        &self,
        user_id: i64,
        password: &Password,
        hash: &str,
    ) -> Result<bool, AuthError> {
        match self.hasher.verify(password, hash).await {
            Ok(matched) => Ok(matched),
            Err(PasswordError::MalformedHash) => {
                tracing::error!(user_id, "stored password hash is malformed; login refused");
                self.hasher.verify_dummy(password).await?;
                Ok(false)
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Replaces `old` with a hash at the current parameters, unless the password was changed
    /// concurrently (the update only matches the hash that was verified).
    async fn rehash(&self, id: i64, old: &str, password: &Password) -> Result<(), AuthError> {
        let fresh = self.hasher.hash(password).await?;
        user::Entity::update_many()
            .col_expr(user::Column::PasswordHash, Expr::value(fresh))
            .filter(user::Column::Id.eq(id))
            .filter(user::Column::PasswordHash.eq(old))
            .exec(&self.db)
            .await?;
        Ok(())
    }
}
