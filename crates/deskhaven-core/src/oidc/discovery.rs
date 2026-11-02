//! Provider metadata (discovery document + JWKS), cached per provider for [`CACHE_TTL`].
//!
//! A stale entry is refetched on next use; an ID token signed by a key the cached JWKS lacks
//! (key rotation) forces one refetch, at most once per [`MIN_REFRESH`] per provider, so
//! forged `kid`s cannot make deskhaven hammer the IdP. A failed fetch is remembered for
//! [`NEGATIVE_TTL`]: during an IdP outage, logins fail at once instead of each waiting for
//! its own timeout. Provider writes drop both. Discovery checks that the document's issuer
//! equals the configured one; the endpoints it names are checked against the provider's
//! [`EndpointPolicy`].
//!
//! Failure reasons name the kind of failure only (status, transport, parse, policy), never
//! upstream bodies, which can echo anything.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use openidconnect::core::CoreProviderMetadata;
use openidconnect::{DiscoveryError, IssuerUrl};

use super::http::{FetchError, HttpFetch, ProviderHttp};
use super::url_policy::EndpointPolicy;
use crate::clock::SharedClock;

/// How long fetched metadata is used before it is fetched again.
pub(crate) const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
/// Minimum age of an entry before a key-rotation refetch.
pub(crate) const MIN_REFRESH: Duration = Duration::from_secs(60);
/// How long a failed fetch is answered from memory.
pub const NEGATIVE_TTL: Duration = Duration::from_secs(30);

/// Metadata of one provider, checked: issuer as configured, usable endpoints.
pub(crate) struct Discovered {
    pub(crate) metadata: CoreProviderMetadata,
    pub(crate) policy: EndpointPolicy,
    fetched: Duration,
}

/// A recent failure: when, and the reason.
struct Failed {
    at: Duration,
    reason: String,
}

pub(crate) struct Discovery {
    fetch: Arc<dyn HttpFetch>,
    clock: SharedClock,
    entries: RwLock<HashMap<i64, Arc<Discovered>>>,
    failures: RwLock<HashMap<i64, Failed>>,
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
            failures: RwLock::default(),
        }
    }

    pub(crate) fn fetcher(&self) -> &dyn HttpFetch {
        self.fetch.as_ref()
    }

    pub(crate) fn forget(&self, provider_id: i64) {
        write(&self.entries).remove(&provider_id);
        write(&self.failures).remove(&provider_id);
    }

    /// The provider's metadata; `Err` is a loggable reason (no secrets, no upstream text).
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
        if let Some(reason) = self.recent_failure(provider_id, now) {
            return Err(format!("{reason} (remembered)"));
        }
        match self.fetch(issuer, now).await {
            Ok(entry) => {
                let entry = Arc::new(entry);
                write(&self.failures).remove(&provider_id);
                write(&self.entries).insert(provider_id, Arc::clone(&entry));
                Ok(entry)
            }
            Err(reason) => {
                let failed = Failed {
                    at: now,
                    reason: reason.clone(),
                };
                write(&self.failures).insert(provider_id, failed);
                Err(reason)
            }
        }
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

    fn recent_failure(&self, provider_id: i64, now: Duration) -> Option<String> {
        let failures = self.failures.read().unwrap_or_else(PoisonError::into_inner);
        let failed = failures.get(&provider_id)?;
        (now.saturating_sub(failed.at) < NEGATIVE_TTL).then(|| failed.reason.clone())
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
            .map_err(|err| format!("discovery failed: {}", discovery_kind(&err)))?;
        check_endpoints(&metadata, policy)?;
        Ok(Discovered {
            metadata,
            policy,
            fetched: now,
        })
    }
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
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

/// What went wrong, without the document's or response's content.
fn discovery_kind(err: &DiscoveryError<FetchError>) -> String {
    match err {
        DiscoveryError::Request(fetch) => fetch_kind(fetch),
        DiscoveryError::Response(status, _, _) => format!("HTTP status {status}"),
        DiscoveryError::Parse(_) => "unparsable document".to_owned(),
        DiscoveryError::UrlParse(_) => "bad URL".to_owned(),
        DiscoveryError::Validation(_) => {
            "the document failed validation (issuer mismatch?)".to_owned()
        }
        _ => "other discovery error".to_owned(),
    }
}

/// Our own transport errors carry no upstream text; the reqwest error is reduced to its
/// kind (its message can contain the URL, which is fine, but nothing else is needed).
pub(crate) fn fetch_kind(err: &FetchError) -> String {
    match err {
        FetchError::Transport(inner) if inner.is_timeout() => "request timed out".to_owned(),
        FetchError::Transport(inner) if inner.is_connect() => "connection failed".to_owned(),
        FetchError::Transport(_) => "request failed".to_owned(),
        other => other.to_string(),
    }
}
