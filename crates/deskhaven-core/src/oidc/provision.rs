//! Step 3 of the identity resolution (see `identity`): a new active `oidc` account for an
//! identity nobody has, with `auto_provision` on. Username by the `username` rules, `admin`
//! when `admin_group` is among the group claim's values, membership in the existing
//! UI-managed groups the claim names (groups are never created from claims). The account
//! is brand new, so no cached membership view can be stale (no group commit hook needed).

use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::user_identity::LinkedVia;
use deskhaven_db::entity::{group, group_member, oidc_provider};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};
use serde_json::json;

use super::OidcService;
use super::claims::Asserted;
use super::error::{CallbackError, Failure};
use super::identity::{Resolution, audit_login, insert_identity, stored_email};
use super::username;
use crate::users::lookup::username_taken;
use crate::users::{Credential, NewUser, Role, UserError};

/// Longest display name taken from the `name` claim (the user service's limit).
const MAX_DISPLAY_NAME_CHARS: usize = 128;

pub(super) async fn provision(
    svc: &OidcService,
    tx: &DatabaseTransaction,
    provider: &oidc_provider::Model,
    asserted: &Asserted,
) -> Result<Resolution, CallbackError> {
    let Some(username) = free_username(tx, asserted).await? else {
        tracing::warn!(provider = %provider.name, "no free username for a provisioned OIDC account");
        return Ok(Err(Failure::ProvisioningFailed));
    };
    let email = stored_email(asserted);
    let is_admin = provider
        .admin_group
        .as_ref()
        .is_some_and(|admin| asserted.groups.contains(admin));
    let new = NewUser {
        display_name: display_name(asserted.name.as_deref()),
        email_verified: email.is_some(),
        email,
        role: if is_admin { Role::Admin } else { Role::User },
        ..NewUser::new(username, Credential::Oidc)
    };
    let account = match svc.inner.users.create_external(tx, &new).await {
        Ok(account) => account,
        // Taken concurrently (the transaction is unusable now: it is rolled back).
        Err(UserError::Conflict { .. }) => return Ok(Err(Failure::ProvisioningFailed)),
        Err(err) => return Err(CallbackError::Internal(err.into())),
    };
    insert_identity(
        svc,
        tx,
        provider.id,
        account.id,
        asserted,
        LinkedVia::Provisioned,
    )
    .await?;
    join_groups(svc, tx, account.id, &asserted.groups).await?;
    let after = json!({
        "username": account.username,
        "role": if is_admin { "admin" } else { "user" },
        "auth_source": "oidc",
        "provider": provider.name,
        "subject": asserted.subject,
        "linked_via": "provisioned",
    });
    audit_login(svc, tx, "user.create", account.id, after).await?;
    tracing::info!(provider = %provider.name, user_id = account.id, "OIDC account provisioned");
    Ok(Ok(account))
}

async fn free_username(
    tx: &DatabaseTransaction,
    asserted: &Asserted,
) -> Result<Option<String>, CallbackError> {
    let base = username::base(asserted);
    for candidate in username::candidates(&base) {
        if !username_taken(tx, &candidate).await? {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

/// The `name` claim without control characters, cut to the user service's limit; blank
/// leaves the username as the display name.
fn display_name(name: Option<&str>) -> String {
    name.unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_DISPLAY_NAME_CHARS)
        .collect()
}

async fn join_groups(
    svc: &OidcService,
    tx: &DatabaseTransaction,
    user_id: i64,
    names: &[String],
) -> Result<(), CallbackError> {
    if names.is_empty() {
        return Ok(());
    }
    let groups = group::Entity::find()
        .filter(group::Column::Name.is_in(names.iter().cloned()))
        .filter(group::Column::ManagedBy.eq(ManagedBy::Ui))
        .all(tx)
        .await?;
    for group in groups {
        group_member::ActiveModel {
            group_id: Set(group.id),
            user_id: Set(user_id),
            created_at: Set(svc.inner.clock.now()),
        }
        .insert(tx)
        .await?;
    }
    Ok(())
}
