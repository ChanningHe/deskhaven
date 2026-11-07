//! Which account a verified identity logs into (06 §3.4, M4-09), inside the callback's
//! write transaction:
//!
//! 1. A linked identity `(provider, sub)`: its account (the stored email is refreshed).
//!    Linked `explicit` (by the account holder) or `provisioned` (the IdP created the
//!    account): nothing is re-checked; the identity replaces the password login. Linked by
//!    `email`: the email-link conditions of step 2 are checked again at every login, not
//!    only at link time (a lowered ceiling, TOTP enabled, a promotion, a changed and so
//!    unverified address, an IdP no longer asserting the address would otherwise be
//!    skipped); failing, the login is refused ([`Failure::NoLinkedAccount`]: the holder
//!    links it explicitly after signing in with the account). A disabled account is refused
//!    whatever the source.
//! 2. Otherwise, an account with the asserted email (case-insensitively): linked (`email`)
//!    only when all hold: the token says `email_verified: true` (missing = false) for the
//!    account's address, the account's own address is verified, its role is within
//!    `auth.oidc.link_email_max_role`, it has no TOTP (whatever the ceiling: an OIDC login
//!    does not ask for it) and it is not an LDAP account (the directory decides on it: an
//!    OIDC login would outlive its removal there). Else refused
//!    ([`Failure::NoLinkedAccount`]). Never provisioned alongside it, never matched by
//!    username.
//! 3. Otherwise, with `auto_provision`: a new account (`provision`).
//! 4. Otherwise refused.
//!
//! OIDC logins ask for no local second factor, and `auth.tfa.enforce` constrains password
//! logins only: a linked identity replaces the password login, and the IdP's MFA policy
//! applies to it.
//!
//! Emails: only an address the IdP asserts verified is ever stored, on the account and on the
//! identity (also when refreshed). An unverified address could be anything the user typed at
//! the IdP; stored, it would hold the real owner's address (unique index) and become their
//! link target later.
//!
//! Known, accepted: step 2's refusal versus step 3's provisioning tells someone who can log
//! in at the IdP whether an address belongs to a deskhaven account (refusing is the clearer
//! experience for the real owner).

use deskhaven_config::settings::{AUTH_OIDC_LINK_EMAIL_MAX_ROLE, LinkEmailMaxRole};
use deskhaven_db::entity::user_identity::{self, LinkedVia};
use deskhaven_db::entity::{oidc_provider, user};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, QueryFilter};

use super::OidcService;
use super::claims::Asserted;
use super::error::{CallbackError, Failure};
use super::provision::provision;
use crate::users::lookup::{email_norm, find_by_email, is_valid_email};
use crate::users::{AuthSource, Role, Status};

pub(super) type Resolution = Result<user::Model, Failure>;

pub(crate) async fn resolve(
    svc: &OidcService,
    tx: &DatabaseTransaction,
    provider: &oidc_provider::Model,
    asserted: &Asserted,
) -> Result<Resolution, CallbackError> {
    let ceiling = svc
        .inner
        .settings
        .snapshot()
        .get(&AUTH_OIDC_LINK_EMAIL_MAX_ROLE);
    if let Some((via, linked)) = linked_account(tx, provider.id, asserted).await? {
        if via == LinkedVia::Email && !email_link_holds(&linked, asserted, ceiling) {
            return Ok(Err(Failure::NoLinkedAccount));
        }
        return Ok(active(linked));
    }
    let by_email = match asserted.email.as_deref() {
        Some(email) => find_by_email(tx, email).await?,
        None => None,
    };
    if let Some(account) = by_email {
        if !email_link_holds(&account, asserted, ceiling) {
            return Ok(Err(Failure::NoLinkedAccount));
        }
        let account = match active(account) {
            Ok(account) => account,
            refused => return Ok(refused),
        };
        insert_identity(svc, tx, provider.id, account.id, asserted, LinkedVia::Email).await?;
        return Ok(Ok(account));
    }
    if !provider.auto_provision {
        return Ok(Err(Failure::NoLinkedAccount));
    }
    provision(svc, tx, provider, asserted).await
}

/// How an existing identity was linked, and its account; its stored email follows the
/// token.
async fn linked_account(
    tx: &DatabaseTransaction,
    provider_id: i64,
    asserted: &Asserted,
) -> Result<Option<(LinkedVia, user::Model)>, CallbackError> {
    let found = user_identity::Entity::find()
        .filter(user_identity::Column::ProviderId.eq(provider_id))
        .filter(user_identity::Column::Subject.eq(&asserted.subject))
        .one(tx)
        .await?;
    let Some(identity) = found else {
        return Ok(None);
    };
    let email = stored_email(asserted);
    if identity.email != email || identity.email_verified != email.is_some() {
        let mut changed: user_identity::ActiveModel = identity.clone().into();
        changed.email_verified = Set(email.is_some());
        changed.email = Set(email);
        changed.update(tx).await?;
    }
    // The foreign key cascades on account deletion, so the account exists.
    let account = user::Entity::find_by_id(identity.user_id).one(tx).await?;
    Ok(account.map(|account| (identity.linked_via, account)))
}

/// The address deskhaven keeps from a token: only one the IdP asserts verified and the user
/// service accepts. An unverified one is never stored (anyone may type any address at an
/// IdP with self-service profiles; stored, it would squat the real owner's address).
pub(super) fn stored_email(asserted: &Asserted) -> Option<String> {
    asserted
        .verified_email()
        .filter(|e| is_valid_email(e))
        .map(str::to_owned)
}

/// The email-link conditions (rule 3 of M4-09), all of them: the IdP asserts the account's
/// address verified, the account's own address is verified, its role is within `ceiling`,
/// it has no TOTP (whatever the ceiling) and it is not an LDAP account.
fn email_link_holds(account: &user::Model, asserted: &Asserted, ceiling: LinkEmailMaxRole) -> bool {
    let same_address = asserted
        .verified_email()
        .and_then(email_norm)
        .is_some_and(|norm| account.email_norm.as_deref() == Some(norm.as_str()));
    same_address
        && account.email_verified
        && within(account.role, ceiling)
        && account.totp_enabled_at.is_none()
        && account.auth_source != AuthSource::Ldap
}

/// Whether `role` is at most `ceiling` (`off` admits none).
fn within(role: Role, ceiling: LinkEmailMaxRole) -> bool {
    let rank = |role: Role| match role {
        Role::User => 1,
        Role::Admin => 2,
        Role::Owner => 3,
    };
    let limit = match ceiling {
        LinkEmailMaxRole::Off => 0,
        LinkEmailMaxRole::User => 1,
        LinkEmailMaxRole::Admin => 2,
        LinkEmailMaxRole::Owner => 3,
    };
    rank(role) <= limit
}

fn active(account: user::Model) -> Resolution {
    match account.status {
        Status::Active => Ok(account),
        Status::Disabled => Err(Failure::Disabled),
        Status::Unverified => Err(Failure::Unverified),
    }
}

pub(super) async fn insert_identity(
    svc: &OidcService,
    tx: &DatabaseTransaction,
    provider_id: i64,
    user_id: i64,
    asserted: &Asserted,
    linked_via: LinkedVia,
) -> Result<user_identity::Model, CallbackError> {
    let email = stored_email(asserted);
    let row = user_identity::ActiveModel {
        user_id: Set(user_id),
        provider_id: Set(provider_id),
        subject: Set(asserted.subject.clone()),
        email_verified: Set(email.is_some()),
        email: Set(email),
        created_at: Set(svc.inner.clock.now()),
        linked_via: Set(linked_via),
        ..Default::default()
    };
    Ok(row.insert(tx).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_admits_its_role_and_those_below() {
        let all = [Role::User, Role::Admin, Role::Owner];
        let admitted = |ceiling| all.into_iter().filter(|r| within(*r, ceiling)).count();

        assert_eq!(admitted(LinkEmailMaxRole::Off), 0);
        assert_eq!(admitted(LinkEmailMaxRole::User), 1);
        assert_eq!(admitted(LinkEmailMaxRole::Admin), 2);
        assert_eq!(admitted(LinkEmailMaxRole::Owner), 3);
        assert!(!within(Role::Owner, LinkEmailMaxRole::Admin));
    }
}
