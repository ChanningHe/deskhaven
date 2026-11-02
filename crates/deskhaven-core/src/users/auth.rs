//! Local password login (06 §3.1, §3.6).

use deskhaven_db::entity::user;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::{AuthError, AuthSource, Inner, MAX_PASSWORD_LEN, Password, Status, User, UserService};

impl UserService {
    /// Verifies a local username / password pair.
    ///
    /// Unknown usernames, malformed usernames, over-long passwords, external (LDAP / OIDC)
    /// accounts and wrong passwords all fail with [`AuthError::InvalidCredentials`] after one
    /// Argon2id computation, so neither the error nor the timing tells whether the account
    /// exists. A hash with outdated parameters is replaced on success.
    ///
    /// Only verifies the password: rate limiting, lockout, 2FA and sessions are the caller's.
    pub async fn authenticate_local(
        &self,
        username: &str,
        password: &Password,
    ) -> Result<User, AuthError> {
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
        if !inner.hasher.verify(password, &hash).await? {
            return Err(AuthError::InvalidCredentials);
        }
        match user.status {
            Status::Active => {}
            Status::Disabled => return Err(AuthError::Disabled),
            Status::Unverified => return Err(AuthError::Unverified),
        }
        if inner.hasher.needs_rehash(&hash) {
            inner.rehash(user.id, &hash, password).await?;
        }
        Ok(user.into())
    }
}

impl Inner {
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
