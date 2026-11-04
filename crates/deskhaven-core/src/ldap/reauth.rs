//! Re-authentication with the current password for self-service changes (console TOTP
//! enrolment): local accounts against their hash, directory accounts against the directory
//! (a bind, nothing provisioned or synchronized). OIDC accounts have no password.

use super::LdapService;
use crate::users::{AuthError, AuthSource, MAX_PASSWORD_LEN, Password, UserError};

impl LdapService {
    /// [`UserError::WrongPassword`] on mismatch, [`UserError::NotLocal`] for an OIDC account,
    /// [`UserError::DirectoryUnavailable`] when the directory cannot answer.
    pub async fn verify_password(
        &self,
        user_id: i64,
        password: &Password,
    ) -> Result<(), UserError> {
        let users = &self.inner.users;
        let account = users.get(user_id).await?;
        match account.auth_source {
            AuthSource::Local => users.verify_password(user_id, password).await,
            AuthSource::Oidc => Err(UserError::NotLocal),
            AuthSource::Ldap => self
                .verify_directory_password(&account.username, password)
                .await
                .map_err(user_error),
        }
    }

    async fn verify_directory_password(
        &self,
        username: &str,
        password: &Password,
    ) -> Result<(), AuthError> {
        self.inner.users.spend_dummy_hash(password).await?;
        let typed = password.expose();
        if typed.is_empty() || password.char_len() > MAX_PASSWORD_LEN {
            return Err(AuthError::InvalidCredentials);
        }
        let settings = self.login_settings().await?;
        match self.verify(&settings, username, typed).await? {
            Some(_) => Ok(()),
            None => Err(AuthError::InvalidCredentials),
        }
    }
}

fn user_error(err: AuthError) -> UserError {
    match err {
        AuthError::InvalidCredentials | AuthError::Disabled | AuthError::Unverified => {
            UserError::WrongPassword
        }
        AuthError::Password(err) => UserError::Password(err),
        AuthError::Database(err) => UserError::Database(err),
        _ => UserError::DirectoryUnavailable,
    }
}
