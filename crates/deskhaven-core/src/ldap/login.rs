//! A directory account's answer comes from the directory alone: there is no local password and
//! no fallback. An unknown username binds a random decoy DN, so wrong password and unknown user
//! give the same answer; disabled / unverified are only told after the bind.
//!
//! Accepted residual risks: a 503 tells that a name is an existing directory account, and anyone
//! can cause one by saturating the directory gate; an unknown name also costs directory round
//! trips on top of the hash. Logs never name a password or the typed username.

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

#[derive(Debug)]
pub struct PasswordLogin {
    pub method: LoginMethod,
    pub result: Result<Authenticated, AuthError>,
}

impl LdapService {
    /// Only verifies: rate limiting, lockout, 2FA and sessions are the caller's.
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

    pub async fn authenticate_password(
        &self,
        username: &str,
        password: &Password,
    ) -> Result<Authenticated, AuthError> {
        self.password_login(username, password).await.result
    }

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
            Err(AuthError::DirectoryUnavailable | AuthError::DirectoryInterrupted) if !known => {
                Err(AuthError::InvalidCredentials)
            }
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

    /// None refuses the login; a broken one is an outage.
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
