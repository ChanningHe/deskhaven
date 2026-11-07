//! Step 2 of an explicit link (see `link`): the IdP's answer is exchanged and verified
//! before the user confirms, so the confirmation page names the IdP identity that will be
//! linked, and the confirmation links exactly that one.
//!
//! The flow stays unconsumed (single use is the confirmation's claim) and bound to the same
//! browser and account. The code is exchanged once: the verified identity, or the failure,
//! is stored on the flow row, and a second verification (a reload) answers from there. Two
//! verifications racing each other: one stores its outcome (conditional update on the
//! unchanged context), the other answers with what was stored.

use deskhaven_db::entity::oidc_flow::{self, Column};
use deskhaven_db::entity::oidc_provider;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::OidcService;
use super::callback::{CallbackParams, Expect};
use super::claims::Asserted;
use super::context;
use super::error::{CallbackError, Failure, LinkFailure};

/// The verified IdP identity a link flow is about to link, for the confirmation page.
/// Values the IdP asserted: escape them before display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkPending {
    /// The provider's display name.
    pub provider: String,
    /// The provider's `sub`.
    pub subject: String,
    /// The address, only when the IdP asserts it verified.
    pub email: Option<String>,
    pub preferred_username: Option<String>,
}

type Verified = Result<LinkPending, LinkFailure>;

impl OidcService {
    /// Exchanges and verifies the IdP's answer of the link flow of `params.state`, for the
    /// browser holding `binding` and the signed-in account `user_id` (anything else:
    /// [`CallbackError::InvalidState`], the flow untouched), once (see the module docs).
    pub async fn verify_link(
        &self,
        params: &CallbackParams<'_>,
        binding: Option<&str>,
        user_id: i64,
    ) -> Result<Verified, CallbackError> {
        let row = self.live_flow(params.state).await?;
        if !Expect::Link(binding, user_id).admits(&row) {
            return Err(CallbackError::InvalidState);
        }
        let provider = oidc_provider::Entity::find_by_id(row.provider_id)
            .one(&self.inner.db)
            .await?
            .ok_or(CallbackError::InvalidState)?;
        if let Some(stored) = stored_outcome(&row, &provider) {
            return Ok(stored);
        }
        let outcome = self.verify_callback(&row, &provider, params).await;
        let written = match &outcome {
            Ok(asserted) => self.store_asserted(&row, asserted).await?,
            Err(failure) => self.store_failure(&row, *failure).await?,
        };
        if !written {
            // Another verification stored its outcome first: answer with that one.
            let again = self.live_flow(params.state).await?;
            return stored_outcome(&again, &provider).ok_or(CallbackError::InvalidState);
        }
        Ok(match outcome {
            Ok(asserted) => Ok(pending(&provider, asserted)),
            Err(failure) => Err(LinkFailure::of(failure)),
        })
    }

    /// Stores the verified identity unless the row changed meanwhile; whether it did.
    async fn store_asserted(
        &self,
        row: &oidc_flow::Model,
        asserted: &Asserted,
    ) -> Result<bool, CallbackError> {
        let context = context::with_asserted(&row.context, asserted)
            .ok_or_else(|| CallbackError::Internal("corrupt OIDC flow context".into()))?;
        let updated = oidc_flow::Entity::update_many()
            .col_expr(Column::Context, Expr::value(context))
            .filter(Column::Id.eq(row.id))
            .filter(Column::Context.eq(row.context.clone()))
            .filter(Column::Error.is_null())
            .exec(&self.inner.db)
            .await?;
        Ok(updated.rows_affected == 1)
    }

    /// Stores the failure unless the row changed meanwhile; whether it did.
    async fn store_failure(
        &self,
        row: &oidc_flow::Model,
        failure: Failure,
    ) -> Result<bool, CallbackError> {
        let updated = oidc_flow::Entity::update_many()
            .col_expr(Column::Error, Expr::value(failure.code()))
            .filter(Column::Id.eq(row.id))
            .filter(Column::Context.eq(row.context.clone()))
            .filter(Column::Error.is_null())
            .exec(&self.inner.db)
            .await?;
        Ok(updated.rows_affected == 1)
    }
}

/// What an earlier verification stored on the row, if any.
fn stored_outcome(row: &oidc_flow::Model, provider: &oidc_provider::Model) -> Option<Verified> {
    if let Some(code) = &row.error {
        let failure = Failure::from_code(code).unwrap_or(Failure::Unavailable);
        return Some(Err(LinkFailure::of(failure)));
    }
    context::link_asserted(&row.context).map(|asserted| Ok(pending(provider, asserted)))
}

fn pending(provider: &oidc_provider::Model, asserted: Asserted) -> LinkPending {
    LinkPending {
        provider: provider.display_name.clone(),
        email: asserted.verified_email().map(str::to_owned),
        subject: asserted.subject,
        preferred_username: asserted.preferred_username,
    }
}
