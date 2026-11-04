//! The deskhaven account of a verified directory user, provisioned at its first login
//! (its role and groups are then synchronized at every login, `sync`).
//!
//! Provisioning (like OIDC's, M2-10): a new active `ldap` account named by the directory's
//! username attribute value that equals the typed username (so it is a valid username,
//! `[A-Za-z0-9._@-]` 1–64, and keys the lockout like the typed one), no local password.
//! The directory's address is stored unverified and only when no other account holds it
//! (directories may let users edit their own address: never a link target, never a
//! squatter). The display name is the directory's, without control, bidi-override or
//! zero-width characters. Accounts are matched by username only; an existing local or
//! OIDC account of that name never reaches the directory (see `login`).

use deskhaven_db::begin_write;
use deskhaven_db::entity::user;

use super::LdapService;
use super::protocol::DirectoryUser;
use super::settings::DirectorySettings;
use crate::text;
use crate::users::lookup::{find_by_email, find_by_username, is_valid_email};
use crate::users::{AuthError, AuthSource, Credential, NewUser, UserError};

/// Longest display name taken from the directory (the user service's limit).
const MAX_DISPLAY_NAME_CHARS: usize = 128;

fn sync_failed(err: impl std::error::Error + Send + Sync + 'static) -> AuthError {
    AuthError::Provisioning(Box::new(err))
}

impl LdapService {
    /// Creates the account of `found` as `username`. A concurrent login that created it
    /// first is fine: its account is used.
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

/// The directory's name without control, bidi-override or zero-width characters (they
/// make a name read as another in the console), cut to the user service's limit; blank
/// leaves the username as the display name.
fn display_name(name: Option<&str>) -> String {
    name.unwrap_or_default()
        .chars()
        .filter(|c| !text::is_dropped(*c))
        .take(MAX_DISPLAY_NAME_CHARS)
        .collect()
}
