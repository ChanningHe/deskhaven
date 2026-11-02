//! Step 2: the IdP's redirect back (see the module docs of `oidc`), once the user confirmed
//! it on the page `preview_callback` describes.
//!
//! The state is claimed before anything else (single use, also against concurrent
//! callbacks); a callback with an unknown, expired or used state does nothing and is not
//! recorded. Once claimed, the flow ends with exactly one outcome on its row: the account
//! (`completed_user_id` and its credential stamp) or a [`Failure`] (`error`).

use std::fmt;

use deskhaven_db::begin_write;
use deskhaven_db::entity::oidc_flow::{self, Column, Flow};
use deskhaven_db::entity::{oidc_provider, user};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

use super::claims::Asserted;
use super::discovery::{Discovered, Freshness};
use super::error::{CallbackError, Failure};
use super::flow::{timestamp, verifier_context};
use super::protocol::{self, ExchangeError, RelyingParty};
use super::{OidcService, context, identity};
use crate::sessions::TokenHash;
use crate::users::{CredentialStamp, User};

/// The query of the redirect (`?code=&state=` or `?error=&state=`). `Debug` shows no value.
pub struct CallbackParams<'a> {
    pub state: Option<&'a str>,
    pub code: Option<&'a str>,
    /// The IdP's `error` (e.g. `access_denied`): the flow fails as [`Failure::Denied`].
    pub error: Option<&'a str>,
}

impl fmt::Debug for CallbackParams<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallbackParams")
            .field("has_state", &self.state.is_some())
            .field("has_code", &self.code.is_some())
            .field("error", &self.error)
            .finish()
    }
}

/// Which kind of login the flow is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowKind {
    /// Started by a RustDesk client, which polls for the session.
    Client { rustdesk_id: String },
    /// Started by the web console (M3-02): the session is the callback caller's business.
    Console,
}

/// The account an OIDC login resolved to, with the stamp to mint its session under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcLogin {
    pub user: User,
    pub stamp: CredentialStamp,
}

/// The outcome of a claimed flow, for the reply page and the login event.
#[derive(Debug)]
pub struct CallbackReport {
    pub flow: FlowKind,
    /// The provider's name (the client's `op`).
    pub provider: String,
    /// The ID token's `sub`, once verified.
    pub subject: Option<String>,
    pub outcome: Result<OidcLogin, Failure>,
}

impl OidcService {
    pub async fn complete_callback(
        &self,
        params: &CallbackParams<'_>,
    ) -> Result<CallbackReport, CallbackError> {
        let row = self.claim(params.state).await?;
        let flow = flow_kind(&row)?;
        let provider = oidc_provider::Entity::find_by_id(row.provider_id)
            .one(&self.inner.db)
            .await?
            .ok_or(CallbackError::InvalidState)?;
        let (subject, outcome) = match self.verify_callback(&row, &provider, params).await {
            Ok(asserted) => match self.resolve(&row, &provider, &asserted).await {
                Ok(outcome) => (Some(asserted.subject), outcome),
                Err(err) => {
                    // Best effort: let the poller stop instead of waiting for the expiry.
                    if let Err(record) = self.record_failure(row.id, Failure::Unavailable).await {
                        tracing::warn!(flow_id = row.id, error = %record, "cannot record the OIDC flow failure");
                    }
                    return Err(err);
                }
            },
            Err(failure) => {
                self.record_failure(row.id, failure).await?;
                (None, Err(failure))
            }
        };
        Ok(CallbackReport {
            flow,
            provider: provider.name,
            subject,
            outcome,
        })
    }

    /// The unconsumed, unexpired flow of `state`, read only.
    pub(crate) async fn live_flow(
        &self,
        state: Option<&str>,
    ) -> Result<oidc_flow::Model, CallbackError> {
        let hash = state
            .and_then(TokenHash::of_presented)
            .ok_or(CallbackError::InvalidState)?;
        let now = timestamp(self.inner.now_ms())?;
        oidc_flow::Entity::find()
            .filter(Column::StateHash.eq(hash.as_str()))
            .filter(Column::ConsumedAt.is_null())
            .filter(Column::ExpiresAt.gt(now))
            .one(&self.inner.db)
            .await?
            .ok_or(CallbackError::InvalidState)
    }

    /// Marks the live flow of `state` consumed and returns it.
    async fn claim(&self, state: Option<&str>) -> Result<oidc_flow::Model, CallbackError> {
        // Read first: a state that matches nothing (the unauthenticated, unlimited case)
        // costs one indexed read, never a write lock (SQLite locks the database for an
        // UPDATE even when it matches no row).
        let live = self.live_flow(state).await?;
        let now = timestamp(self.inner.now_ms())?;
        let claimed = oidc_flow::Entity::update_many()
            .col_expr(Column::ConsumedAt, Expr::value(now))
            .filter(Column::Id.eq(live.id))
            .filter(Column::ConsumedAt.is_null())
            .exec_with_returning(&self.inner.db)
            .await?;
        claimed
            .into_iter()
            .next()
            .ok_or(CallbackError::InvalidState)
    }

    /// The identity the IdP asserts for this flow, or why there is none.
    async fn verify_callback(
        &self,
        row: &oidc_flow::Model,
        provider: &oidc_provider::Model,
        params: &CallbackParams<'_>,
    ) -> Result<Asserted, Failure> {
        if !provider.enabled {
            return Err(Failure::Unavailable);
        }
        let code = match (params.error, params.code) {
            (None, Some(code)) if !code.is_empty() => code,
            _ => return Err(Failure::Denied),
        };
        let redirect_uri = self
            .inner
            .redirect_uri
            .as_ref()
            .ok_or(Failure::Unavailable)?;
        let verifier = self.open_verifier(row).ok_or(Failure::Unavailable)?;
        let secret = self.client_secret(provider).map_err(|err| {
            tracing::error!(provider = %provider.name, error = %err, "cannot open the OIDC client secret");
            Failure::Unavailable
        })?;
        let rp = RelyingParty {
            client_id: &provider.client_id,
            client_secret: secret.as_deref().map(String::as_str),
            redirect_uri,
        };
        let discovered = self.discovered(provider, Freshness::Cached).await?;
        let fetch = self.inner.discovery.fetcher();
        let token = protocol::exchange(&discovered, fetch, &rp, code, &verifier)
            .await
            .map_err(|err| exchange_failure(&provider.name, err))?;
        let claims =
            match protocol::verify(&discovered, &rp, &self.inner.clock, &token, &row.nonce) {
                Err(err) if protocol::is_unknown_key(&err) => {
                    let fresh = self.discovered(provider, Freshness::Refresh).await?;
                    protocol::verify(&fresh, &rp, &self.inner.clock, &token, &row.nonce)
                }
                other => other,
            }
            .map_err(|err| {
                tracing::warn!(provider = %provider.name, error = %err, "OIDC ID token rejected");
                Failure::InvalidToken
            })?;
        Asserted::from_claims(&claims, provider.group_claim.as_deref()).ok_or_else(|| {
            tracing::warn!(provider = %provider.name, "OIDC ID token has no usable subject");
            Failure::InvalidToken
        })
    }

    async fn discovered(
        &self,
        provider: &oidc_provider::Model,
        freshness: Freshness,
    ) -> Result<std::sync::Arc<Discovered>, Failure> {
        self.inner
            .discovery
            .get(provider.id, &provider.issuer, freshness)
            .await
            .map_err(|reason| {
                tracing::warn!(provider = %provider.name, %reason, "OIDC discovery failed");
                Failure::Unavailable
            })
    }

    fn open_verifier(&self, row: &oidc_flow::Model) -> Option<String> {
        let context = verifier_context(&row.state_hash);
        let opened = self.inner.crypto.open(&context, &row.pkce_verifier_enc);
        match opened.map(|plain| String::from_utf8(plain.to_vec())) {
            Ok(Ok(verifier)) => Some(verifier),
            _ => {
                tracing::error!(
                    flow_id = row.id,
                    "cannot open the PKCE verifier of an OIDC flow"
                );
                None
            }
        }
    }

    /// Links or creates the account and completes the flow in one transaction; a refusal is
    /// rolled back and recorded on its own.
    async fn resolve(
        &self,
        row: &oidc_flow::Model,
        provider: &oidc_provider::Model,
        asserted: &Asserted,
    ) -> Result<Result<OidcLogin, Failure>, CallbackError> {
        let tx = begin_write(&self.inner.db).await?;
        match identity::resolve(self, &tx, provider, asserted).await? {
            Ok(account) => {
                let login = self.record_success(&tx, row, account).await?;
                tx.commit().await?;
                Ok(Ok(login))
            }
            Err(failure) => {
                drop(tx);
                self.record_failure(row.id, failure).await?;
                Ok(Err(failure))
            }
        }
    }

    async fn record_success<C: ConnectionTrait>(
        &self,
        conn: &C,
        row: &oidc_flow::Model,
        account: user::Model,
    ) -> Result<OidcLogin, CallbackError> {
        let stamp = CredentialStamp::of(&account);
        let context = context::with_stamp(&row.context, stamp).ok_or_else(|| corrupt(row.id))?;
        oidc_flow::Entity::update_many()
            .col_expr(Column::CompletedUserId, Expr::value(account.id))
            .col_expr(Column::Context, Expr::value(context))
            .filter(Column::Id.eq(row.id))
            .exec(conn)
            .await?;
        Ok(OidcLogin {
            user: account.into(),
            stamp,
        })
    }

    async fn record_failure(&self, flow_id: i64, failure: Failure) -> Result<(), CallbackError> {
        oidc_flow::Entity::update_many()
            .col_expr(Column::Error, Expr::value(failure.code()))
            .filter(Column::Id.eq(flow_id))
            .exec(&self.inner.db)
            .await?;
        Ok(())
    }
}

fn flow_kind(row: &oidc_flow::Model) -> Result<FlowKind, CallbackError> {
    match row.flow {
        Flow::Console => Ok(FlowKind::Console),
        Flow::Client => context::client_device(&row.context)
            .map(|device| FlowKind::Client {
                rustdesk_id: device.rustdesk_id,
            })
            .ok_or_else(|| corrupt(row.id)),
    }
}

fn exchange_failure(provider: &str, err: ExchangeError) -> Failure {
    match err {
        ExchangeError::Unavailable(reason) => {
            tracing::warn!(provider, %reason, "OIDC code exchange failed");
            Failure::Unavailable
        }
        ExchangeError::Invalid(reason) => {
            tracing::warn!(provider, %reason, "OIDC token response rejected");
            Failure::InvalidToken
        }
    }
}

fn corrupt(flow_id: i64) -> CallbackError {
    tracing::error!(flow_id, "OIDC flow context is unreadable");
    CallbackError::Internal("corrupt OIDC flow context".into())
}
