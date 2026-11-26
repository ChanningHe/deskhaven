//! The confirmation page names the verified identity, and the confirmation links exactly that one;
//! the email plays no part. Once linked, an OIDC login asks for no local second factor: the
//! account's safety is the IdP account's.

use deskhaven_db::begin_write;
use deskhaven_db::entity::oidc_flow::{self, Flow};
use deskhaven_db::entity::user_identity::{self, LinkedVia};
use deskhaven_db::entity::{oidc_provider, user};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter,
    SqlErr,
};

use super::OidcService;
use super::callback::Expect;
use super::claims::Asserted;
use super::context;
use super::error::{CallbackError, LinkFailure, OidcError};
use super::flow::{BrowserBinding, ConsoleFlowStart, FlowRow};
use super::identities::{Identity, IdentityChange, locked_account};
use super::identity::insert_identity;
use crate::console_audit::TxGuard;
use crate::sessions::SessionToken;
use crate::users::{AuthSource, Status, User};

#[derive(Debug)]
pub struct LinkReport {
    pub provider: String,
    pub subject: Option<String>,
    pub outcome: Result<Identity, LinkFailure>,
}

type Linking = Result<Identity, LinkFailure>;

impl OidcService {
    pub async fn begin_link(
        &self,
        provider: &str,
        user_id: i64,
    ) -> Result<ConsoleFlowStart, OidcError> {
        self.check_link(provider, user_id).await?;
        let binding = SessionToken::generate()?;
        let row = FlowRow {
            flow: Flow::Link,
            context: context::for_link(&binding.hash(), user_id),
            client_code_hash: None,
        };
        let url = self.begin(provider, row).await?;
        Ok(ConsoleFlowStart {
            url,
            binding: BrowserBinding::new(binding),
        })
    }

    /// Run before re-authenticating, so a refusal spends no code.
    pub async fn check_link(&self, provider: &str, user_id: i64) -> Result<(), OidcError> {
        let found = self
            .enabled_provider(provider)
            .await?
            .ok_or(OidcError::UnknownProvider)?;
        let account = user::Entity::find_by_id(user_id)
            .one(&self.inner.db)
            .await?
            .ok_or(OidcError::LinkRefused(LinkFailure::AccountInactive))?;
        if let Some(refusal) = refusal(&account) {
            return Err(OidcError::LinkRefused(refusal));
        }
        // One linked by email may be the very identity the user now links explicitly.
        let existing = provider_identity(&self.inner.db, user_id, found.id).await?;
        if existing.is_some_and(|row| row.linked_via != LinkedVia::Email) {
            return Err(OidcError::LinkRefused(LinkFailure::ProviderLinked));
        }
        Ok(())
    }

    /// A flow not verified yet, or of another browser or account, stays untouched. Single use.
    pub async fn complete_link(
        &self,
        state: Option<&str>,
        binding: Option<&str>,
        user_id: i64,
        guard: TxGuard<IdentityChange>,
    ) -> Result<LinkReport, CallbackError> {
        let live = self.live_flow(state).await?;
        if live.error.is_some() || context::link_asserted(&live.context).is_none() {
            return Err(CallbackError::InvalidState);
        }
        let row = self.claim(state, Expect::Link(binding, user_id)).await?;
        let report = self.finish_link(&row, user_id, guard).await;
        let deleted = oidc_flow::Entity::delete_by_id(row.id)
            .exec(&self.inner.db)
            .await;
        if let Err(err) = deleted {
            tracing::warn!(flow_id = row.id, error = %err, "cannot delete a completed OIDC link flow");
        }
        report
    }

    async fn finish_link(
        &self,
        row: &oidc_flow::Model,
        user_id: i64,
        guard: TxGuard<IdentityChange>,
    ) -> Result<LinkReport, CallbackError> {
        let provider = oidc_provider::Entity::find_by_id(row.provider_id)
            .one(&self.inner.db)
            .await?
            .ok_or(CallbackError::InvalidState)?;
        // Exactly the identity the confirmation page showed.
        let asserted = context::link_asserted(&row.context).ok_or(CallbackError::InvalidState)?;
        // Disabled since the verification: no new way in through it.
        let outcome = match provider.enabled {
            true => {
                self.link_verified(&provider, user_id, &asserted, guard)
                    .await?
            }
            false => Err(LinkFailure::Unavailable),
        };
        let subject = Some(asserted.subject);
        match &outcome {
            Ok(identity) => {
                tracing::info!(user_id, provider = %provider.name, identity_id = identity.id, "OIDC identity linked");
            }
            // Refused by the link rules (not the IdP): worth a look; never the subject.
            Err(
                failure @ (LinkFailure::IdentityTaken
                | LinkFailure::ProviderLinked
                | LinkFailure::DirectoryAccount
                | LinkFailure::AccountInactive),
            ) => {
                tracing::warn!(user_id, provider = %provider.name, reason = failure.code(), "OIDC identity link refused");
            }
            Err(_) => {}
        }
        Ok(LinkReport {
            provider: provider.name,
            subject,
            outcome,
        })
    }

    async fn link_verified(
        &self,
        provider: &oidc_provider::Model,
        user_id: i64,
        asserted: &Asserted,
        guard: TxGuard<IdentityChange>,
    ) -> Result<Linking, CallbackError> {
        let tx = begin_write(&self.inner.db).await?;
        let linked = match self.link_in(&tx, provider, user_id, asserted).await? {
            Ok(linked) => linked,
            Err(refusal) => return Ok(Err(refusal)),
        };
        let Some((account, identity)) = linked else {
            return Ok(Ok(self.already_linked(&tx, provider, asserted).await?));
        };
        let change = IdentityChange {
            user: User::from(account),
            identity,
        };
        match guard.check(&tx, None, Some(&change)).await {
            Ok(()) => {}
            Err(crate::console_audit::GuardError::Refused(_)) => {
                return Ok(Err(LinkFailure::AccountInactive));
            }
            Err(crate::console_audit::GuardError::Database(err)) => return Err(err.into()),
        }
        tx.commit().await?;
        Ok(Ok(change.identity))
    }

    async fn link_in(
        &self,
        tx: &DatabaseTransaction,
        provider: &oidc_provider::Model,
        user_id: i64,
        asserted: &Asserted,
    ) -> Result<Result<Option<(user::Model, Identity)>, LinkFailure>, CallbackError> {
        let Some(account) = locked_account(tx, user_id).await? else {
            return Ok(Err(LinkFailure::AccountInactive));
        };
        if let Some(refusal) = refusal(&account) {
            return Ok(Err(refusal));
        }
        let existing = by_key(tx, provider.id, &asserted.subject).await?;
        let row = match existing {
            Some(row) if row.user_id != user_id => return Ok(Err(LinkFailure::IdentityTaken)),
            Some(row) if row.linked_via != LinkedVia::Email => return Ok(Ok(None)),
            Some(row) => {
                let mut upgraded = row.into_active_model();
                upgraded.linked_via = Set(LinkedVia::Explicit);
                upgraded.update(tx).await?
            }
            None => {
                if provider_identity(tx, user_id, provider.id).await?.is_some() {
                    return Ok(Err(LinkFailure::ProviderLinked));
                }
                let inserted = insert_identity(
                    self,
                    tx,
                    provider.id,
                    user_id,
                    asserted,
                    LinkedVia::Explicit,
                )
                .await;
                match inserted {
                    Ok(row) => row,
                    // Linked to another account concurrently (unique `(provider, sub)`).
                    Err(err)
                        if matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) =>
                    {
                        return Ok(Err(LinkFailure::IdentityTaken));
                    }
                    Err(err) => return Err(err.into()),
                }
            }
        };
        Ok(Ok(Some((account, Identity::of(row, provider)))))
    }

    async fn already_linked(
        &self,
        tx: &DatabaseTransaction,
        provider: &oidc_provider::Model,
        asserted: &Asserted,
    ) -> Result<Identity, CallbackError> {
        let row = by_key(tx, provider.id, &asserted.subject)
            .await?
            .ok_or(CallbackError::InvalidState)?;
        Ok(Identity::of(row, provider))
    }
}

fn refusal(account: &user::Model) -> Option<LinkFailure> {
    if account.auth_source == AuthSource::Ldap {
        return Some(LinkFailure::DirectoryAccount);
    }
    (account.status != Status::Active).then_some(LinkFailure::AccountInactive)
}

async fn by_key<C: sea_orm::ConnectionTrait>(
    conn: &C,
    provider_id: i64,
    subject: &str,
) -> Result<Option<user_identity::Model>, sea_orm::DbErr> {
    user_identity::Entity::find()
        .filter(user_identity::Column::ProviderId.eq(provider_id))
        .filter(user_identity::Column::Subject.eq(subject))
        .one(conn)
        .await
}

pub(super) async fn provider_identity<C: sea_orm::ConnectionTrait>(
    conn: &C,
    user_id: i64,
    provider_id: i64,
) -> Result<Option<user_identity::Model>, sea_orm::DbErr> {
    user_identity::Entity::find()
        .filter(user_identity::Column::UserId.eq(user_id))
        .filter(user_identity::Column::ProviderId.eq(provider_id))
        .one(conn)
        .await
}
