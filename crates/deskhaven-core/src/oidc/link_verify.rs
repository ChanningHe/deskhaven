//! Step 2 of an explicit link (see `link`): the IdP's answer is exchanged and verified
//! before the user confirms, so the confirmation page names the IdP identity that will be
//! linked, and the confirmation links exactly that one.
//!
//! The flow stays unconsumed (single use is the confirmation's claim) and bound to the same
//! browser and account. The code is exchanged once:
//!
//! 1. A verification first claims the flow: a conditional update marks it `verifying_at`
//!    (unchanged context, no stored failure). A request that loses the claim does not ask
//!    the IdP; it gets [`CallbackError::Verifying`] and asks again shortly.
//! 2. The winner exchanges the code and stores the verified identity, or the failure, in
//!    place of the mark. A second verification (a reload) answers from there.
//! 3. A claim that never stored anything (the process died mid-exchange) is taken over
//!    after [`VERIFY_GRACE`], well inside the flow's own lifetime.

use std::time::Duration;

use deskhaven_db::entity::oidc_flow::{self, Column};
use deskhaven_db::entity::oidc_provider;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::OidcService;
use super::callback::{CallbackParams, Expect};
use super::claims::Asserted;
use super::context;
use super::error::{CallbackError, Failure, LinkFailure};

/// How long a started verification blocks others: longer than an exchange can take
/// (discovery and token request, each bounded by the HTTP timeouts).
pub const VERIFY_GRACE: Duration = Duration::from_secs(60);

/// The verified IdP identity a link flow is about to link, for the confirmation page.
/// Values the IdP asserted: escape them before display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkPending {
    /// The provider's name.
    pub provider_name: String,
    /// The provider's display name.
    pub provider: String,
    /// The provider's `sub`.
    pub subject: String,
    /// The address, only when the IdP asserts it verified.
    pub email: Option<String>,
    pub preferred_username: Option<String>,
    /// This call exchanged the code (false: answered from an earlier verification).
    pub exchanged: bool,
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
        let claimed = self.claim_verification(&row).await?;
        let outcome = self.verify_callback(&row, &provider, params).await;
        match &outcome {
            Ok(asserted) => self.store_asserted(&row, &claimed, asserted).await?,
            Err(failure) => self.store_failure(row.id, &claimed, *failure).await?,
        }
        Ok(match outcome {
            Ok(asserted) => Ok(LinkPending {
                exchanged: true,
                ..pending(&provider, asserted)
            }),
            Err(failure) => Err(LinkFailure::of(failure)),
        })
    }

    /// Marks the flow as being verified; the claimed context. [`CallbackError::Verifying`]
    /// while another verification holds a recent claim, or when it wins the race.
    async fn claim_verification(&self, row: &oidc_flow::Model) -> Result<String, CallbackError> {
        let now = self.inner.now_ms();
        let grace = i64::try_from(VERIFY_GRACE.as_millis()).unwrap_or(i64::MAX);
        if context::verifying_since(&row.context).is_some_and(|at| now - at < grace) {
            return Err(CallbackError::Verifying);
        }
        let claimed = context::with_verifying(&row.context, now).ok_or_else(corrupt)?;
        let updated = oidc_flow::Entity::update_many()
            .col_expr(Column::Context, Expr::value(claimed.clone()))
            .filter(Column::Id.eq(row.id))
            .filter(Column::Context.eq(row.context.clone()))
            .filter(Column::Error.is_null())
            .exec(&self.inner.db)
            .await?;
        match updated.rows_affected {
            1 => Ok(claimed),
            _ => Err(CallbackError::Verifying),
        }
    }

    /// Stores the verified identity in place of the claim (`row` is the unclaimed context).
    async fn store_asserted(
        &self,
        row: &oidc_flow::Model,
        claimed: &str,
        asserted: &Asserted,
    ) -> Result<(), CallbackError> {
        let context = context::with_asserted(&row.context, asserted).ok_or_else(corrupt)?;
        self.replace_claim(row.id, claimed, Column::Context, context)
            .await
    }

    async fn store_failure(
        &self,
        id: i64,
        claimed: &str,
        failure: Failure,
    ) -> Result<(), CallbackError> {
        self.replace_claim(id, claimed, Column::Error, failure.code().to_owned())
            .await
    }

    /// Sets `column` while the flow still holds this claim; a claim taken over after the
    /// grace period means another verification answers now.
    async fn replace_claim(
        &self,
        id: i64,
        claimed: &str,
        column: Column,
        value: String,
    ) -> Result<(), CallbackError> {
        let updated = oidc_flow::Entity::update_many()
            .col_expr(column, Expr::value(value))
            .filter(Column::Id.eq(id))
            .filter(Column::Context.eq(claimed))
            .filter(Column::Error.is_null())
            .exec(&self.inner.db)
            .await?;
        match updated.rows_affected {
            1 => Ok(()),
            _ => Err(CallbackError::Verifying),
        }
    }
}

fn corrupt() -> CallbackError {
    CallbackError::Internal("corrupt OIDC flow context".into())
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
        provider_name: provider.name.clone(),
        provider: provider.display_name.clone(),
        email: asserted.verified_email().map(str::to_owned),
        subject: asserted.subject,
        preferred_username: asserted.preferred_username,
        exchanged: false,
    }
}
