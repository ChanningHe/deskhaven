//! The directory's address is stored unverified and only when no other account holds it: users
//! may edit their own address in a directory, so it is never a link target. Accounts are matched
//! by username only.

use deskhaven_db::begin_write;
use deskhaven_db::entity::user;

use super::LdapService;
use super::protocol::DirectoryUser;
use super::settings::DirectorySettings;
use crate::text;
use crate::users::lookup::{find_by_email, find_by_username, is_valid_email};
use crate::users::{AuthError, AuthSource, Credential, NewUser, UserError};

const MAX_DISPLAY_NAME_CHARS: usize = 128;

fn sync_failed(err: impl std::error::Error + Send + Sync + 'static) -> AuthError {
    AuthError::Provisioning(Box::new(err))
}

impl LdapService {
    /// A concurrent login that created it first is fine: its account is used.
    pub(crate) async fn provision(
        &self,
        settings: &DirectorySettings,
        username: &str,
        found: &DirectoryUser,
    ) -> Result<user::Model, AuthError> {
        let inner = &self.inner;
        let tx = begin_write(&inner.db).await?;
        let email = match found.email.as_deref().filter(|e| is_valid_email(e)) {
            Some(email) if find_by_email(&tx, email).await?.is_none() => Some(email.to_owned()),
            _ => None,
        };
        let new = NewUser {
            display_name: display_name(found.display_name.as_deref()),
            email,
            ..NewUser::new(username, Credential::Ldap)
        };
        match inner.users.create_external(&tx, &new).await {
            Ok(created) => {
                tx.commit().await?;
                tracing::info!(directory = %settings.name, user_id = created.id, "LDAP account provisioned");
                Ok(created)
            }
            Err(UserError::Conflict { .. }) => {
                drop(tx);
                let raced = find_by_username(&inner.db, username).await?;
                raced
                    .filter(|a| a.auth_source == AuthSource::Ldap)
                    .ok_or(AuthError::InvalidCredentials)
            }
            Err(err) => Err(sync_failed(err)),
        }
    }
}

/// Bidi-override and zero-width characters would make a name read as another in the console.
fn display_name(name: Option<&str>) -> String {
    text::clean_import(name.unwrap_or_default(), MAX_DISPLAY_NAME_CHARS)
}
