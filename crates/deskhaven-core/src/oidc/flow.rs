//! Step 1: a new flow row and the authorization URL (see the module docs of `oidc`).

use std::fmt;

use deskhaven_db::entity::oidc_flow::{self, Flow};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition};
use url::Url;

use super::context;
use super::discovery::Freshness;
use super::error::OidcError;
use super::protocol::{self, FlowSecrets, RelyingParty};
use super::{FLOW_TTL, OidcService};
use crate::jobs::{Doomed, Plain, Purge};
use crate::sessions::{ClientDevice, SessionToken};

/// The polling `code` of a client flow. Exposed once; `Debug` never shows it.
pub struct ClientCode(SessionToken);

impl ClientCode {
    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

impl fmt::Debug for ClientCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ClientCode(<redacted>)")
    }
}

/// `{code, url}` of `/api/oidc/auth`.
#[derive(Debug)]
pub struct ClientFlowStart {
    pub code: ClientCode,
    /// The IdP's authorization endpoint with the request parameters.
    pub url: Url,
}

/// The value that ties a console flow to the browser that started it (a cookie). Exposed
/// once; `Debug` never shows it.
pub struct BrowserBinding(SessionToken);

impl BrowserBinding {
    pub(super) fn new(token: SessionToken) -> Self {
        Self(token)
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

impl fmt::Debug for BrowserBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BrowserBinding(<redacted>)")
    }
}

/// A started console login: send the browser to `url` with `binding` in a cookie.
#[derive(Debug)]
pub struct ConsoleFlowStart {
    pub url: Url,
    /// Must come back with the completion (see `OidcService::complete_console`).
    pub binding: BrowserBinding,
}

/// What differs between the flow kinds at the start.
pub(super) struct FlowRow {
    pub(super) flow: Flow,
    pub(super) context: serde_json::Value,
    pub(super) client_code_hash: Option<String>,
}

impl OidcService {
    /// Starts a client login at the enabled provider named `provider` (the client's `op`)
    /// for `device`, which the session will be bound to.
    pub async fn begin_client(
        &self,
        provider: &str,
        device: &ClientDevice,
    ) -> Result<ClientFlowStart, OidcError> {
        device.check().map_err(|_| OidcError::InvalidDevice)?;
        let code = SessionToken::generate()?;
        let row = FlowRow {
            flow: Flow::Client,
            context: context::for_client(device),
            client_code_hash: Some(code.hash().as_str().to_owned()),
        };
        let url = self.begin(provider, row).await?;
        Ok(ClientFlowStart {
            code: ClientCode(code),
            url,
        })
    }

    /// Starts a web console login at the enabled provider named `provider`. The flow only
    /// completes with the returned binding (`complete_console`), so a callback opened in
    /// another browser (a forwarded link, an attacker's flow) does nothing.
    pub async fn begin_console(&self, provider: &str) -> Result<ConsoleFlowStart, OidcError> {
        let binding = SessionToken::generate()?;
        let row = FlowRow {
            flow: Flow::Console,
            context: context::for_console(&binding.hash()),
            client_code_hash: None,
        };
        let url = self.begin(provider, row).await?;
        Ok(ConsoleFlowStart {
            url,
            binding: BrowserBinding(binding),
        })
    }

    /// Stores the flow row and returns the IdP's authorization URL.
    pub(super) async fn begin(&self, provider: &str, row: FlowRow) -> Result<Url, OidcError> {
        let redirect_uri = self
            .inner
            .redirect_uri
            .as_ref()
            .ok_or(OidcError::NotConfigured)?;
        let provider = self
            .enabled_provider(provider)
            .await?
            .ok_or(OidcError::UnknownProvider)?;
        let discovered = self
            .inner
            .discovery
            .get(provider.id, &provider.issuer, Freshness::Cached)
            .await
            .map_err(|reason| {
                tracing::warn!(provider = %provider.name, %reason, "OIDC discovery failed");
                OidcError::Unavailable(reason)
            })?;
        let secret = self.client_secret(&provider)?;
        let state = SessionToken::generate()?;
        let nonce = SessionToken::generate()?;
        let verifier = SessionToken::generate()?;
        let state_hash = state.hash();
        let sealed = self.inner.crypto.seal(
            &verifier_context(state_hash.as_str()),
            verifier.expose().as_bytes(),
        )?;
        let now = self.inner.now_ms();
        let ttl = i64::try_from(FLOW_TTL.as_millis()).unwrap_or(i64::MAX);
        oidc_flow::ActiveModel {
            state_hash: Set(state_hash.as_str().to_owned()),
            client_code_hash: Set(row.client_code_hash),
            provider_id: Set(provider.id),
            pkce_verifier_enc: Set(sealed),
            nonce: Set(nonce.expose().to_owned()),
            flow: Set(row.flow),
            context: Set(row.context.to_string()),
            created_at: Set(timestamp(now)?),
            expires_at: Set(timestamp(now.saturating_add(ttl))?),
            completed_user_id: Set(None),
            consumed_at: Set(None),
            error: Set(None),
            ..Default::default()
        }
        .insert(&self.inner.db)
        .await?;
        let rp = RelyingParty {
            client_id: &provider.client_id,
            client_secret: secret.as_deref().map(String::as_str),
            redirect_uri,
        };
        let secrets = FlowSecrets {
            state: state.expose(),
            nonce: nonce.expose(),
            pkce_verifier: verifier.expose(),
        };
        Ok(protocol::authorization_url(
            &discovered,
            &rp,
            &provider.scopes,
            &secrets,
        ))
    }

    /// Deletes expired flows in batches; returns how many. The expiry job (M2-20) deletes
    /// [`expired_flows`] with its own batch settings.
    pub async fn cleanup_flows(&self) -> Result<u64, OidcError> {
        let now = timestamp(self.inner.now_ms())?;
        let deleted = Purge::default()
            .run(&self.inner.db, &expired_flows(now), &mut Plain)
            .await?;
        Ok(deleted)
    }
}

/// Flows expired at `now`: what cleanup deletes.
pub(crate) fn expired_flows(now: TimestampMs) -> Doomed<oidc_flow::Entity> {
    use oidc_flow::Column as C;
    Doomed {
        key: C::Id,
        order: C::ExpiresAt,
        cond: Condition::all().add(C::ExpiresAt.lte(now)),
    }
}

/// Sealing context of a flow's PKCE verifier: the row's unique state hash (the id does not
/// exist before the insert).
pub(crate) fn verifier_context(state_hash: &str) -> String {
    format!("oidc_flows.pkce_verifier_enc:{state_hash}")
}

pub(crate) fn timestamp(ms: i64) -> Result<TimestampMs, sea_orm::DbErr> {
    time::from_millis(ms)
        .ok_or_else(|| sea_orm::DbErr::Custom(format!("timestamp {ms} out of range")))
}
