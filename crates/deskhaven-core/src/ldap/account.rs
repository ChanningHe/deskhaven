//! The deskhaven account of a verified directory user: provisioned at the first login,
//! its role and groups synchronized at every login (`mapping`).
//!
//! Provisioning (like OIDC's, M2-10): a new active `ldap` account named by the directory's
//! username attribute value that equals the typed username (so it is a valid username,
//! `[A-Za-z0-9._@-]` 1–64, and keys the lockout like the typed one), no local password.
//! The directory's address is stored unverified and only when no other account holds it
//! (directories may let users edit their own address: never a link target, never a
//! squatter). The display name is the directory's, without control characters. Accounts
//! are matched by username only; an existing local or OIDC account of that name never
//! reaches the directory (see `login`).

use std::collections::BTreeSet;

use deskhaven_db::begin_write;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::user;

use super::LdapService;
use super::mapping;
use super::protocol::DirectoryUser;
use super::settings::DirectorySettings;
use crate::users::lookup::{find_by_email, find_by_username, is_valid_email};
use crate::users::{AuthError, AuthSource, Credential, NewUser, Role, User, UserError};

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

    /// Applies the directory's role and groups to `account` (see `mapping`).
    pub(crate) async fn sync(
        &self,
        settings: &DirectorySettings,
        account: user::Model,
        group_dns: &[String],
    ) -> Result<User, AuthError> {
        let id = account.id;
        let mut user = User::from(account);
        if let Some(admin) = mapping::is_admin(settings, group_dns) {
            let wanted = if admin { Role::Admin } else { Role::User };
            if user.role != Role::Owner && user.role != wanted {
                user = self
                    .inner
                    .users
                    .set_role(id, wanted)
                    .await
                    .map_err(sync_failed)?;
                tracing::info!(directory = %settings.name, user_id = id, role = ?wanted, "role synchronized from the directory");
            }
        }
        if let Some(names) = mapping::group_names(settings, group_dns) {
            self.sync_groups(id, &names).await?;
        }
        Ok(user)
    }

    /// Membership in exactly the console-managed groups named `names` (lowercased).
    async fn sync_groups(&self, user_id: i64, names: &BTreeSet<String>) -> Result<(), AuthError> {
        let groups = &self.inner.groups;
        let all = groups.list().await.map_err(sync_failed)?;
        let current: BTreeSet<i64> = groups
            .groups_of_user(user_id)
            .await
            .map_err(sync_failed)?
            .into_iter()
            .map(|g| g.id)
            .collect();
        for group in all.iter().filter(|g| g.managed_by == ManagedBy::Ui) {
            let wanted = names.contains(&group.name.to_lowercase());
            let member = current.contains(&group.id);
            if wanted && !member {
                groups
                    .add_member(group.id, user_id)
                    .await
                    .map_err(sync_failed)?;
            } else if member && !wanted {
                groups
                    .remove_member(group.id, user_id)
                    .await
                    .map_err(sync_failed)?;
            }
        }
        Ok(())
    }
}

/// The directory's name without control characters, cut to the user service's limit;
/// blank leaves the username as the display name.
fn display_name(name: Option<&str>) -> String {
    name.unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_DISPLAY_NAME_CHARS)
        .collect()
}
