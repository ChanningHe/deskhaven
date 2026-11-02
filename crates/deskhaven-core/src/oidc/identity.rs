//! Which account a verified identity logs into (06 §3.4), inside the callback's write
//! transaction:
//!
//! 1. A linked identity `(provider, sub)`: its account (the stored email is refreshed).
//! 2. Otherwise, an account with the asserted email (case-insensitively): linked only when
//!    `email_verified` is `true` (missing = false) **and** `auth.oidc.link_verified_email` is
//!    on; else refused ([`Failure::NoLinkedAccount`]: link it from the console). Never
//!    provisioned alongside it, never matched by username.
//! 3. Otherwise, with `auto_provision`: a new active `oidc` account (`username` rules), the
//!    asserted email (if well formed) with its verification state, `admin` when
//!    `admin_group` is among the group claim's values, and membership in the existing
//!    UI-managed groups the claim names (groups are never created from claims). The account
//!    is brand new, so no cached membership view can be stale (no group commit hook needed).
//! 4. Otherwise refused.
//!
//! Linked accounts must be active (disabled / unverified are refused, not linked). Group
//! mapping and the admin flag apply at provisioning only; later changes are the console's.

use deskhaven_config::settings::AUTH_OIDC_LINK_VERIFIED_EMAIL;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::{group, group_member, oidc_provider, user, user_identity};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use super::OidcService;
use super::claims::Asserted;
use super::error::{CallbackError, Failure};
use super::username;
use crate::users::lookup::{find_by_email, is_valid_email, username_taken};
use crate::users::{Credential, NewUser, Role, Status, UserError};

/// Longest display name taken from the `name` claim (the user service's limit).
const MAX_DISPLAY_NAME_CHARS: usize = 128;

type Resolution = Result<user::Model, Failure>;

pub(crate) async fn resolve(
    svc: &OidcService,
    tx: &DatabaseTransaction,
    provider: &oidc_provider::Model,
    asserted: &Asserted,
) -> Result<Resolution, CallbackError> {
    if let Some(linked) = linked_account(tx, provider.id, asserted).await? {
        return Ok(active(linked));
    }
    let by_email = match asserted.email.as_deref() {
        Some(email) => find_by_email(tx, email).await?,
        None => None,
    };
    if let Some(account) = by_email {
        let may_link = asserted.verified_email().is_some()
            && svc
                .inner
                .settings
                .snapshot()
                .get(&AUTH_OIDC_LINK_VERIFIED_EMAIL);
        if !may_link {
            return Ok(Err(Failure::NoLinkedAccount));
        }
        let account = match active(account) {
            Ok(account) => account,
            refused => return Ok(refused),
        };
        link(svc, tx, provider.id, account.id, asserted).await?;
        return Ok(Ok(account));
    }
    if !provider.auto_provision {
        return Ok(Err(Failure::NoLinkedAccount));
    }
    provision(svc, tx, provider, asserted).await
}

/// The account of an existing identity; its stored email follows the token.
async fn linked_account(
    tx: &DatabaseTransaction,
    provider_id: i64,
    asserted: &Asserted,
) -> Result<Option<user::Model>, CallbackError> {
    let found = user_identity::Entity::find()
        .filter(user_identity::Column::ProviderId.eq(provider_id))
        .filter(user_identity::Column::Subject.eq(&asserted.subject))
        .one(tx)
        .await?;
    let Some(identity) = found else {
        return Ok(None);
    };
    let verified = asserted.email_verified == Some(true);
    if identity.email != asserted.email || identity.email_verified != verified {
        let mut changed: user_identity::ActiveModel = identity.clone().into();
        changed.email = Set(asserted.email.clone());
        changed.email_verified = Set(verified);
        changed.update(tx).await?;
    }
    // The foreign key cascades on account deletion, so the account exists.
    Ok(user::Entity::find_by_id(identity.user_id).one(tx).await?)
}

fn active(account: user::Model) -> Resolution {
    match account.status {
        Status::Active => Ok(account),
        Status::Disabled => Err(Failure::Disabled),
        Status::Unverified => Err(Failure::Unverified),
    }
}

async fn link(
    svc: &OidcService,
    tx: &DatabaseTransaction,
    provider_id: i64,
    user_id: i64,
    asserted: &Asserted,
) -> Result<(), CallbackError> {
    user_identity::ActiveModel {
        user_id: Set(user_id),
        provider_id: Set(provider_id),
        subject: Set(asserted.subject.clone()),
        email: Set(asserted.email.clone()),
        email_verified: Set(asserted.email_verified == Some(true)),
        created_at: Set(svc.inner.clock.now()),
        ..Default::default()
    }
    .insert(tx)
    .await?;
    Ok(())
}

async fn provision(
    svc: &OidcService,
    tx: &DatabaseTransaction,
    provider: &oidc_provider::Model,
    asserted: &Asserted,
) -> Result<Resolution, CallbackError> {
    let Some(username) = free_username(tx, asserted).await? else {
        tracing::warn!(provider = %provider.name, "no free username for a provisioned OIDC account");
        return Ok(Err(Failure::ProvisioningFailed));
    };
    let email = asserted.email.clone().filter(|e| is_valid_email(e));
    let is_admin = provider
        .admin_group
        .as_ref()
        .is_some_and(|admin| asserted.groups.contains(admin));
    let new = NewUser {
        display_name: display_name(asserted.name.as_deref()),
        email_verified: email.is_some() && asserted.email_verified == Some(true),
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
    link(svc, tx, provider.id, account.id, asserted).await?;
    join_groups(svc, tx, account.id, &asserted.groups).await?;
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
