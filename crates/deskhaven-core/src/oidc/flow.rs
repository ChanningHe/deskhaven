//! Step 1: a new flow row and the authorization URL (see the module docs of `oidc`).

use std::fmt;

use deskhaven_db::entity::oidc_flow::{self, Flow};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use url::Url;

use super::context;
use super::discovery::Freshness;
use super::error::OidcError;
use super::protocol::{self, FlowSecrets, RelyingParty};
use super::{FLOW_TTL, OidcService};
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

impl OidcService {
    /// Starts a client login at the enabled provider named `provider` (the client's `op`)
    /// for `device`, which the session will be bound to.
    pub async fn begin_client(
        &self,
        provider: &str,
        device: &ClientDevice,
    ) -> Result<ClientFlowStart, OidcError> {
        let redirect_uri = self
            .inner
            .redirect_uri
            .as_ref()
            .ok_or(OidcError::NotConfigured)?;
        device.check().map_err(|_| OidcError::InvalidDevice)?;
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
        let code = SessionToken::generate()?;
        let state_hash = state.hash();
        let context = context::for_client(device);
        let sealed = self.inner.crypto.seal(
            &verifier_context(state_hash.as_str()),
            verifier.expose().as_bytes(),
        )?;
        let now = self.inner.now_ms();
        let ttl = i64::try_from(FLOW_TTL.as_millis()).unwrap_or(i64::MAX);
        oidc_flow::ActiveModel {
            state_hash: Set(state_hash.as_str().to_owned()),
            client_code_hash: Set(Some(code.hash().as_str().to_owned())),
            provider_id: Set(provider.id),
            pkce_verifier_enc: Set(sealed),
            nonce: Set(nonce.expose().to_owned()),
            flow: Set(Flow::Client),
            context: Set(context.to_string()),
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
        let url = protocol::authorization_url(&discovered, &rp, &provider.scopes, &secrets);
        Ok(ClientFlowStart {
            code: ClientCode(code),
            url,
        })
    }

    /// Deletes expired flows; returns how many (periodic job, M2-20).
    pub async fn cleanup_flows(&self) -> Result<u64, OidcError> {
        let now = timestamp(self.inner.now_ms())?;
        let result = oidc_flow::Entity::delete_many()
            .filter(oidc_flow::Column::ExpiresAt.lte(now))
            .exec(&self.inner.db)
            .await?;
        Ok(result.rows_affected)
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
