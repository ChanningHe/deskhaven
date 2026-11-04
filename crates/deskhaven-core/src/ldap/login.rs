//! Password logins, local or directory (06 §3.5): [`LdapService::password_login`] is what
//! the login endpoints call instead of `UserService::authenticate_local`.
//!
//! Routing, by the account the username names (case-insensitively):
//!
//! - a local or OIDC account, a malformed username, or no account while no directory is
//!   enabled: `authenticate_local` (OIDC accounts and unknown names fail there after the
//!   dummy hash). The directory is never asked. Login method `password`.
//! - an `ldap` account, or no account while a directory is enabled: the directory (login
//!   method `ldap`). Its answer is final: **there is no local password and no fallback**.
//!
//! A directory login spends one dummy Argon2id verification first (the same cost as a
//! local login), refuses empty and over-long passwords before connecting, then runs the
//! directory steps (`verify`: connect -> service bind -> search
//! `(&<user_filter>(<username_attr>=<escaped username>))` -> user bind -> groups) under a
//! bound on concurrent directory logins and one total timeout. An unknown username binds a
//! random decoy DN instead, so wrong password and unknown user give the same answer. A
//! refused bind, a missing or ambiguous entry and a directory value that is not the typed
//! username are all [`AuthError::InvalidCredentials`]; disabled / unverified are only told
//! after the bind.
//!
//! When the directory cannot answer (unreachable, timed out, saturated, refusing the
//! service account, misconfigured): an existing `ldap` account gets
//! [`AuthError::DirectoryUnavailable`] (APIs: 503, not counted); a name without an account
//! gets [`AuthError::InvalidCredentials`], the counted 401 a wrong local password gets, so
//! the status code never tells whether a name has an account. **Accepted residual risk:**
//! timing still differs, since an unknown name costs directory round trips (or the
//! timeout during an outage) on top of the hash that a local account's check costs.
//!
//! Logs name the directory and the failing step, never a password or the typed username.

use deskhaven_db::entity::user;

use super::LdapService;
use super::settings::DirectorySettings;
use super::verify::VerifiedUser;
use crate::login_events::LoginMethod;
use crate::users::lookup::find_by_username;
use crate::users::{
    AuthError, AuthSource, Authenticated, CredentialStamp, MAX_PASSWORD_LEN, Password, Status,
    username_norm,
};

/// A password check and how it was made (for the login event).
#[derive(Debug)]
pub struct PasswordLogin {
    /// `ldap` when the directory decided, `password` for a local check.
    pub method: LoginMethod,
    pub result: Result<Authenticated, AuthError>,
}

impl LdapService {
    /// Verifies a username / password pair against the account's source (see the module
    /// docs). Only verifies: rate limiting, lockout, 2FA and sessions are the caller's.
    pub async fn password_login(&self, username: &str, password: &Password) -> PasswordLogin {
        match self.route(username).await {
            Ok(Some(account)) => PasswordLogin {
                method: LoginMethod::Ldap,
                result: self.directory_login(username, password, account).await,
            },
            Ok(None) => PasswordLogin {
                method: LoginMethod::Password,
                result: self
                    .inner
                    .users
                    .authenticate_local(username, password)
                    .await,
            },
            Err(err) => PasswordLogin {
                method: LoginMethod::Password,
                result: Err(err),
            },
        }
    }

    /// [`password_login`](Self::password_login)'s result alone.
    pub async fn authenticate_password(
        &self,
        username: &str,
        password: &Password,
    ) -> Result<Authenticated, AuthError> {
        self.password_login(username, password).await.result
    }

    /// `Some(account)` (`None`: not provisioned yet) when the directory decides.
    async fn route(&self, username: &str) -> Result<Option<Option<user::Model>>, AuthError> {
        if username_norm(username).is_none() {
            return Ok(None);
        }
        let account = find_by_username(&self.inner.db, username).await?;
        let directory = match &account {
            Some(account) => account.auth_source == AuthSource::Ldap,
            None => self.has_enabled_directory().await?,
        };
        Ok(directory.then_some(account))
    }

    async fn directory_login(
        &self,
        username: &str,
        password: &Password,
        account: Option<user::Model>,
    ) -> Result<Authenticated, AuthError> {
        let known = account.is_some();
        match self.directory_check(username, password, account).await {
            // No status oracle for names without an account (see the module docs).
            Err(AuthError::DirectoryUnavailable) if !known => Err(AuthError::InvalidCredentials),
            outcome => outcome,
        }
    }

    async fn directory_check(
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
        let settings = self.login_settings().await?;
        let Some(found) = self.verify(&settings, username, typed).await? else {
            return Err(AuthError::InvalidCredentials);
        };
        let account = match account {
            Some(account) => account,
            None => self.provision(&settings, &found.name, &found.user).await?,
        };
        self.complete(&settings, account, &found).await
    }

    /// The enabled directory; none refuses the login, a broken one is an outage.
    pub(crate) async fn login_settings(&self) -> Result<DirectorySettings, AuthError> {
        match self.enabled_settings().await {
            Ok(Some(settings)) => Ok(settings),
            Ok(None) => {
                tracing::info!("LDAP account login refused: no directory is enabled");
                Err(AuthError::InvalidCredentials)
            }
            Err(err) => {
                tracing::error!(error = %err, "cannot load the enabled directory");
                Err(AuthError::DirectoryUnavailable)
            }
        }
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
