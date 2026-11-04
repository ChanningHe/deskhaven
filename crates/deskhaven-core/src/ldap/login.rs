//! Password logins, local or directory (06 §3.5): [`LdapService::authenticate_password`]
//! is what the login endpoints call instead of `UserService::authenticate_local`.
//!
//! Routing, by the account the username names (case-insensitively):
//!
//! - a local or OIDC account, a malformed username, or no account while no directory is
//!   enabled: `authenticate_local` (OIDC accounts and unknown names fail there after the
//!   dummy hash). The directory is never asked.
//! - an `ldap` account, or no account while a directory is enabled: the directory. Its
//!   answer is final: **there is no local password and no fallback**. An unreachable or
//!   misconfigured directory is [`AuthError::DirectoryUnavailable`] (APIs: 503, refunded),
//!   never a local check.
//!
//! A directory login spends one dummy Argon2id verification first (the same cost as a
//! local login, and the hasher's slots bound concurrent directory logins), refuses empty
//! and over-long passwords before connecting, then: connect -> service bind -> search
//! `(&<user_filter>(<username_attr>=<escaped username>))` -> user bind -> groups. An
//! unknown username binds a random decoy DN instead, so wrong password and unknown user
//! give the same answer after the same round trips. A refused bind, a missing or ambiguous
//! entry and a directory value that is not the typed username are all
//! [`AuthError::InvalidCredentials`]; disabled / unverified are only told after the bind.
//! Logs name the directory and the failing step, never a password or the typed username.

use deskhaven_db::entity::user;

use super::LdapService;
use super::directory::{DirectoryFault, Session};
use super::protocol::{self, DirectoryUser, Lookup};
use super::settings::DirectorySettings;
use crate::users::lookup::find_by_username;
use crate::users::{
    AuthError, AuthSource, Authenticated, CredentialStamp, MAX_PASSWORD_LEN, Password, Status,
    username_norm,
};

impl LdapService {
    /// Verifies a username / password pair against the account's source (see the module
    /// docs). Only verifies: rate limiting, lockout, 2FA and sessions are the caller's.
    pub async fn authenticate_password(
        &self,
        username: &str,
        password: &Password,
    ) -> Result<Authenticated, AuthError> {
        let users = &self.inner.users;
        if username_norm(username).is_none() {
            return users.authenticate_local(username, password).await;
        }
        let account = find_by_username(&self.inner.db, username).await?;
        let directory = match &account {
            Some(account) => account.auth_source == AuthSource::Ldap,
            None => self.has_enabled_directory().await?,
        };
        if !directory {
            return users.authenticate_local(username, password).await;
        }
        self.directory_login(username, password, account).await
    }

    async fn directory_login(
        &self,
        username: &str,
        password: &Password,
        account: Option<user::Model>,
    ) -> Result<Authenticated, AuthError> {
        self.inner.users.spend_dummy_hash(password).await?;
        let typed = password.expose();
        if typed.is_empty() || password.char_len() > MAX_PASSWORD_LEN {
            return Err(AuthError::InvalidCredentials);
        }
        let settings = match self.enabled_settings().await {
            Ok(Some(settings)) => settings,
            Ok(None) => {
                tracing::info!("LDAP account login refused: no directory is enabled");
                return Err(AuthError::InvalidCredentials);
            }
            Err(err) => {
                tracing::error!(error = %err, "cannot load the enabled directory");
                return Err(AuthError::DirectoryUnavailable);
            }
        };
        let Some(found) = self.verify(&settings, username, typed).await? else {
            return Err(AuthError::InvalidCredentials);
        };
        let Some(name) = found.user.username_matching(username) else {
            tracing::warn!(directory = %settings.name, "directory username attribute does not match the login name; refused");
            return Err(AuthError::InvalidCredentials);
        };
        let account = match account {
            Some(account) => account,
            None => self.provision(&settings, name, &found.user).await?,
        };
        self.complete(&settings, account, &found).await
    }

    /// Connects, runs the steps, closes. `None`: the directory refused the login.
    async fn verify(
        &self,
        settings: &DirectorySettings,
        username: &str,
        password: &str,
    ) -> Result<Option<VerifiedUser>, AuthError> {
        let mut session = self
            .inner
            .connector
            .connect(&settings.target)
            .await
            .map_err(|fault| unavailable(settings, "connect", &fault))?;
        let verified = steps(session.as_mut(), settings, username, password).await;
        session.close().await;
        verified
    }

    async fn complete(
        &self,
        settings: &DirectorySettings,
        account: user::Model,
        found: &VerifiedUser,
    ) -> Result<Authenticated, AuthError> {
        match account.status {
            Status::Active => {}
            Status::Disabled => return Err(AuthError::Disabled),
            Status::Unverified => return Err(AuthError::Unverified),
        }
        let stamp = CredentialStamp::of(&account);
        let user = self.sync(settings, account, &found.group_dns).await?;
        Ok(Authenticated { user, stamp })
    }
}

/// A directory user whose password the directory accepted, with its group DNs.
struct VerifiedUser {
    user: DirectoryUser,
    group_dns: Vec<String>,
}

async fn steps(
    session: &mut dyn Session,
    settings: &DirectorySettings,
    username: &str,
    password: &str,
) -> Result<Option<VerifiedUser>, AuthError> {
    let fail = |step| move |fault: DirectoryFault| unavailable(settings, step, &fault);
    protocol::bind_service(session, settings)
        .await
        .map_err(fail("service bind"))?;
    let user = match protocol::find_user(session, settings, username)
        .await
        .map_err(fail("user search"))?
    {
        Lookup::Found(user) => user,
        Lookup::Missing => {
            protocol::decoy_bind(session, settings, password)
                .await
                .map_err(fail("user bind"))?;
            return Ok(None);
        }
        Lookup::Ambiguous => {
            tracing::warn!(directory = %settings.name, "several directory entries match one username; login refused");
            return Ok(None);
        }
    };
    let accepted = protocol::bind_user(session, &user.dn, password)
        .await
        .map_err(fail("user bind"))?;
    if !accepted {
        return Ok(None);
    }
    let group_dns = protocol::groups_of(session, settings, &user)
        .await
        .map_err(fail("group lookup"))?;
    Ok(Some(VerifiedUser { user, group_dns }))
}

fn unavailable(
    settings: &DirectorySettings,
    step: &'static str,
    fault: &DirectoryFault,
) -> AuthError {
    tracing::warn!(directory = %settings.name, step, error = %fault, "directory login failed: directory unavailable or misconfigured");
    AuthError::DirectoryUnavailable
}
