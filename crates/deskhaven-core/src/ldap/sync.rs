//! Applies the directory's role and groups to a directory account at every login
//! (`mapping`). Each change goes through the owning service's guarded write: the account
//! (or membership) is locked and re-read in the write transaction, the guard decides on
//! that committed state, and an audit row (actor `system`) commits with the change.
//! Owners are never changed, also when one became an owner after the login read the
//! account; a change that turns out to be a no-op is dropped without a row. Either way the
//! login continues with the account as that transaction read it (so the second factor is
//! judged on the committed role, e.g. a promotion made while the bind was pending).

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::user;
use sea_orm::ActiveEnum;
use serde_json::json;

use super::LdapService;
use super::mapping;
use super::settings::DirectorySettings;
use crate::console_audit::{Actor, NewConsoleAudit, Refused, TxGuard};
use crate::groups::{GroupError, Membership};
use crate::users::{AccountEdit, AuthError, Role, User, UserError};

/// Guard refusal: the account is an owner (the directory never changes owners).
const OWNER: &str = "ldap_sync_owner";
/// Guard refusal: nothing to change after all.
const UNCHANGED: &str = "ldap_sync_unchanged";

fn sync_failed(err: impl std::error::Error + Send + Sync + 'static) -> AuthError {
    AuthError::Provisioning(Box::new(err))
}

fn system_entry(action: &str, target_type: &str, target_id: i64) -> NewConsoleAudit {
    NewConsoleAudit {
        actor: Actor::System,
        action: action.to_owned(),
        target_type: target_type.to_owned(),
        target_id: target_id.to_string(),
        before: None,
        after: None,
        ip: None,
    }
}

impl LdapService {
    /// The directory's role and groups applied to `account` (see `mapping`).
    pub(crate) async fn sync(
        &self,
        settings: &DirectorySettings,
        account: user::Model,
        group_dns: &[String],
    ) -> Result<User, AuthError> {
        let mut user = User::from(account);
        if let Some(admin) = mapping::is_admin(settings, group_dns) {
            let wanted = if admin { Role::Admin } else { Role::User };
            if user.role != Role::Owner && user.role != wanted {
                user = self.sync_role(settings, user, wanted).await?;
            }
        }
        if let Some(names) = mapping::group_names(settings, group_dns) {
            self.sync_groups(user.id, &names).await?;
        }
        Ok(user)
    }

    async fn sync_role(
        &self,
        settings: &DirectorySettings,
        user: User,
        wanted: Role,
    ) -> Result<User, AuthError> {
        let id = user.id;
        // The account as the edit's transaction read it (locked), for a refused edit: the
        // caller must judge the committed role, not the one read before the bind.
        let committed: Arc<Mutex<Option<User>>> = Arc::default();
        let seen = Arc::clone(&committed);
        let guard = TxGuard::new(&self.inner.audit, move |before: Option<&User>, after| {
            let (Some(before), Some(after)) = (before, after) else {
                return Err(Refused::new(UNCHANGED));
            };
            *seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(before.clone());
            if before.role == Role::Owner {
                return Err(Refused::new(OWNER));
            }
            if before.role == after.role {
                return Err(Refused::new(UNCHANGED));
            }
            Ok(NewConsoleAudit {
                before: Some(json!({ "role": before.role.to_value() })),
                after: Some(json!({ "role": after.role.to_value() })),
                ..system_entry("user.update", "user", id)
            })
        });
        let edit = AccountEdit {
            role: Some(wanted),
            ..AccountEdit::default()
        };
        match self.inner.users.edit(id, edit, guard).await {
            Ok(changed) => {
                tracing::info!(directory = %settings.name, user_id = id, role = ?wanted, "role synchronized from the directory");
                Ok(changed)
            }
            Err(UserError::Refused(_)) => {
                let current = committed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                Ok(current.unwrap_or(user))
            }
            Err(err) => Err(sync_failed(err)),
        }
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
            let changed = match (wanted, member) {
                (true, false) => {
                    let guard = self.member_guard("group.member_add", group.id);
                    groups.add_member_guarded(group.id, user_id, guard).await
                }
                (false, true) => {
                    let guard = self.member_guard("group.member_remove", group.id);
                    groups.remove_member_guarded(group.id, user_id, guard).await
                }
                _ => Ok(()),
            };
            match changed {
                Ok(()) | Err(GroupError::Refused(_)) => {}
                Err(err) => return Err(sync_failed(err)),
            }
        }
        Ok(())
    }

    /// Audits a membership change the directory makes; a no-op (already / not a member
    /// in the transaction) is refused, so it leaves no row.
    fn member_guard(&self, action: &'static str, group_id: i64) -> TxGuard<Membership> {
        TxGuard::new(
            &self.inner.audit,
            move |before: Option<&Membership>, after| {
                let member =
                    |m: &Membership| json!({ "user_id": m.user_id, "username": m.username });
                let changed = before.is_some() != after.is_some();
                if !changed {
                    return Err(Refused::new(UNCHANGED));
                }
                Ok(NewConsoleAudit {
                    before: before.map(member),
                    after: after.map(member),
                    ..system_entry(action, "group", group_id)
                })
            },
        )
    }
}
