//! Provider metadata (discovery document + JWKS), cached per provider for [`CACHE_TTL`].
//!
//! A stale entry is refetched on next use; an ID token signed by a key the cached JWKS lacks
//! (key rotation) forces one refetch, at most once per [`MIN_REFRESH`] per provider, so
//! forged `kid`s cannot make deskhaven hammer the IdP. Provider writes drop the entry.
//! Discovery checks that the document's issuer equals the configured one; the endpoints it
//! names are checked against the provider's [`EndpointPolicy`].

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use openidconnect::IssuerUrl;
use openidconnect::core::CoreProviderMetadata;

use super::http::{HttpFetch, ProviderHttp};
use super::url_policy::EndpointPolicy;
use crate::clock::SharedClock;

/// How long fetched metadata is used before it is fetched again.
pub(crate) const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
/// Minimum age of an entry before a key-rotation refetch.
pub(crate) const MIN_REFRESH: Duration = Duration::from_secs(60);

/// Metadata of one provider, checked: issuer as configured, usable endpoints.
pub(crate) struct Discovered {
    pub(crate) metadata: CoreProviderMetadata,
    pub(crate) policy: EndpointPolicy,
    fetched: Duration,
}

pub(crate) struct Discovery {
    fetch: Arc<dyn HttpFetch>,
    clock: SharedClock,
    entries: RwLock<HashMap<i64, Arc<Discovered>>>,
}

/// Which entry a lookup accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Freshness {
    /// Any entry younger than [`CACHE_TTL`].
    Cached,
    /// Refetch unless the entry is younger than [`MIN_REFRESH`].
    Refresh,
}

impl Discovery {
    pub(crate) fn new(fetch: Arc<dyn HttpFetch>, clock: SharedClock) -> Self {
        Self {
            fetch,
            clock,
            entries: RwLock::default(),
        }
    }

    pub(crate) fn fetcher(&self) -> &dyn HttpFetch {
        self.fetch.as_ref()
    }

    pub(crate) fn forget(&self, provider_id: i64) {
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&provider_id);
    }

    /// The provider's metadata; `Err` is a loggable reason (no secrets in it).
    pub(crate) async fn get(
        &self,
        provider_id: i64,
        issuer: &str,
        freshness: Freshness,
    ) -> Result<Arc<Discovered>, String> {
        let now = self.clock.elapsed();
        if let Some(entry) = self.cached(provider_id, now, freshness) {
            return Ok(entry);
        }
        let entry = Arc::new(self.fetch(issuer, now).await?);
        self.entries
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(provider_id, Arc::clone(&entry));
        Ok(entry)
    }

    fn cached(
        &self,
        provider_id: i64,
        now: Duration,
        freshness: Freshness,
    ) -> Option<Arc<Discovered>> {
        let entries = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        let entry = entries.get(&provider_id)?;
        let age = now.saturating_sub(entry.fetched);
        let max_age = match freshness {
            Freshness::Cached => CACHE_TTL,
            Freshness::Refresh => MIN_REFRESH,
        };
        (age < max_age).then(|| Arc::clone(entry))
    }

    async fn fetch(&self, issuer: &str, now: Duration) -> Result<Discovered, String> {
        // As configured, character for character: the document's `issuer` and the tokens'
        // `iss` must equal it exactly.
        let issuer_url =
            IssuerUrl::new(issuer.to_owned()).map_err(|_| "issuer is not a URL".to_owned())?;
        let policy = EndpointPolicy::for_issuer(issuer_url.url());
        let http = ProviderHttp::new(self.fetch.as_ref(), policy);
        let metadata = CoreProviderMetadata::discover_async(issuer_url, &http)
            .await
            .map_err(|err| format!("discovery failed: {}", error_chain(&err)))?;
        check_endpoints(&metadata, policy)?;
        Ok(Discovered {
            metadata,
            policy,
            fetched: now,
        })
    }
}

/// The browser is sent to the authorization endpoint (it must not be `javascript:` or plain
/// http); the token endpoint is required for the code flow.
fn check_endpoints(metadata: &CoreProviderMetadata, policy: EndpointPolicy) -> Result<(), String> {
    policy
        .check(metadata.authorization_endpoint().url())
        .map_err(|reason| format!("authorization endpoint refused: {reason}"))?;
    let token = metadata
        .token_endpoint()
        .ok_or_else(|| "the discovery document has no token endpoint".to_owned())?;
    policy
        .check(token.url())
        .map_err(|reason| format!("token endpoint refused: {reason}"))
}

/// `err` and its sources, for logs.
pub(crate) fn error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
