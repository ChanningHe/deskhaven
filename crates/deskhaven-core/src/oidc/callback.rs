//! The state is claimed before anything else (single use, also against concurrent callbacks);
//! once claimed, the flow ends with exactly one outcome on its row.

use std::fmt;

use deskhaven_db::begin_write;
use deskhaven_db::entity::oidc_flow::{self, Column, Flow};
use deskhaven_db::entity::{oidc_provider, user};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use subtle::ConstantTimeEq;

use super::claims::Asserted;
use super::discovery::{Discovered, Freshness};
use super::error::{CallbackError, Failure};
use super::flow::{timestamp, verifier_context};
use super::protocol::{self, ExchangeError, RelyingParty};
use super::{OidcService, context, identity};
use crate::sessions::TokenHash;
use crate::users::{CredentialStamp, User};

/// `Debug` shows no value.
pub struct CallbackParams<'a> {
    pub state: Option<&'a str>,
    pub code: Option<&'a str>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowKind {
    Client { rustdesk_id: String },
    Console,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcLogin {
    pub user: User,
    pub stamp: CredentialStamp,
}

#[derive(Debug)]
pub struct CallbackReport {
    pub flow: FlowKind,
    pub provider: String,
    pub subject: Option<String>,
    pub outcome: Result<OidcLogin, Failure>,
}

#[derive(Clone, Copy)]
pub(super) enum Expect<'a> {
    Client,
    Console(Option<&'a str>),
    Link(Option<&'a str>, i64),
}

impl Expect<'_> {
    pub(super) fn admits(self, row: &oidc_flow::Model) -> bool {
        match (self, row.flow) {
            (Self::Client, Flow::Client) => true,
            (Self::Console(presented), Flow::Console) => bound(presented, row),
            (Self::Link(presented, user_id), Flow::Link) => {
                context::link_user(&row.context) == Some(user_id) && bound(presented, row)
            }
            _ => false,
        }
    }
}

fn bound(presented: Option<&str>, row: &oidc_flow::Model) -> bool {
    let (Some(hash), Some(stored)) = (
        presented.and_then(TokenHash::of_presented),
        context::console_binding(&row.context),
    ) else {
        return false;
    };
    bool::from(hash.as_str().as_bytes().ct_eq(stored.as_bytes()))
}

impl OidcService {
    /// A console flow stays untouched here: it completes only through `complete_console`.
    pub async fn complete_callback(
        &self,
        params: &CallbackParams<'_>,
    ) -> Result<CallbackReport, CallbackError> {
        self.complete(params, Expect::Client).await
    }

    /// A missing or different binding leaves the flow untouched: a callback link forwarded to, or
    /// planted in, another browser does nothing.
    pub async fn complete_console(
        &self,
        params: &CallbackParams<'_>,
        binding: Option<&str>,
    ) -> Result<CallbackReport, CallbackError> {
        self.complete(params, Expect::Console(binding)).await
    }

    async fn complete(
        &self,
        params: &CallbackParams<'_>,
        expect: Expect<'_>,
    ) -> Result<CallbackReport, CallbackError> {
        let row = self.claim(params.state, expect).await?;
        let flow = flow_kind(&row)?;
        let report = self.finish(&row, flow, params).await;
        if row.flow == Flow::Console {
            // Nobody polls a console flow: its outcome is in the report.
            let deleted = oidc_flow::Entity::delete_by_id(row.id)
                .exec(&self.inner.db)
                .await;
            if let Err(err) = deleted {
                tracing::warn!(flow_id = row.id, error = %err, "cannot delete a completed console OIDC flow");
            }
        }
        report
    }

    async fn finish(
        &self,
        row: &oidc_flow::Model,
        flow: FlowKind,
        params: &CallbackParams<'_>,
    ) -> Result<CallbackReport, CallbackError> {
        let provider = oidc_provider::Entity::find_by_id(row.provider_id)
            .one(&self.inner.db)
            .await?
            .ok_or(CallbackError::InvalidState)?;
        let (subject, outcome) = match self.verify_callback(row, &provider, params).await {
            Ok(asserted) => match self.resolve(row, &provider, &asserted).await {
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

    pub(super) async fn claim(
        &self,
        state: Option<&str>,
        expect: Expect<'_>,
    ) -> Result<oidc_flow::Model, CallbackError> {
        // Read first: a state that matches nothing (the unauthenticated, unlimited case)
        // costs one indexed read, never a write lock (SQLite locks the database for an
        // UPDATE even when it matches no row).
        let live = self.live_flow(state).await?;
        if !expect.admits(&live) {
            return Err(CallbackError::InvalidState);
        }
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

    pub(super) async fn verify_callback(
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
                let kind = protocol::rejection_kind(&err);
                tracing::warn!(provider = %provider.name, kind, "OIDC ID token rejected");
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
        // Never admitted by a login completion.
        Flow::Link => Err(CallbackError::InvalidState),
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
