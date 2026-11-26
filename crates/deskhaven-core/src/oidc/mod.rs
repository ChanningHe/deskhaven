//! State, polling code and browser binding are stored only as SHA-256; the PKCE verifier is
//! sealed. A client flow's callback only shows which device is about to be signed in and completes
//! on confirmation (see `preview`); a console flow completes only in the browser holding its
//! binding, so a forwarded or planted callback link does nothing.
//!
//! The identity key `(provider_id, sub)` stands for `(issuer, sub)`: a provider's issuer never
//! changes after creation.

mod callback;
mod claims;
mod context;
mod discovery;
mod error;
mod flow;
mod http;
mod identities;
mod identity;
mod link;
mod link_verify;
mod poll;
mod preview;
mod protocol;
mod providers;
mod provision;
mod resolver;
mod url_policy;
mod username;

use std::sync::Arc;
use std::time::Duration;

use sea_orm::DatabaseConnection;
use url::Url;

pub use callback::{CallbackParams, CallbackReport, FlowKind, OidcLogin};
pub use discovery::NEGATIVE_TTL;
pub use error::{
    CallbackError, Failure, IdentityError, LinkFailure, OidcError, PollError, ProviderError,
};
pub(crate) use flow::expired_flows;
pub use flow::{BrowserBinding, ClientCode, ClientFlowStart, ConsoleFlowStart};
pub use http::{
    CONNECT_TIMEOUT, FetchError, FetchFuture, HttpFetch, MAX_RESPONSE_BYTES, REQUEST_TIMEOUT,
    ReqwestFetch,
};
pub use identities::{Identity, IdentityChange};
pub use link::LinkReport;
pub use link_verify::LinkPending;
pub use poll::{ClientLogin, PollOutcome};
pub use preview::{CallbackPreview, DeviceSummary, PreviewTarget};
pub use providers::{
    LoginButton, MAX_ICON_LEN, MAX_NAME_LEN, NewProvider, Provider, ProviderUpdate,
};
pub(crate) use resolver::is_forbidden;
pub use resolver::{Lookup, LookupFuture, system_lookup};
pub use url_policy::MAX_ISSUER_LEN;

use crate::clock::SharedClock;
use crate::crypto::SecretBox;
use crate::settings::SettingsService;
use crate::users::UserService;

use discovery::Discovery;

/// The client gives up after 3 minutes; short so a phished authorization URL goes stale quickly.
pub const FLOW_TTL: Duration = Duration::from_secs(5 * 60);
pub const CALLBACK_PATH: &str = "api/oidc/callback";

#[derive(Clone)]
pub struct OidcService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    crypto: Arc<SecretBox>,
    settings: SettingsService,
    users: UserService,
    clock: SharedClock,
    redirect_uri: Option<Url>,
    discovery: Discovery,
    buttons: providers::ButtonCache,
}

impl OidcService {
    /// Blank `public_url` leaves providers manageable but every login refused, and no login buttons.
    pub fn new(
        db: DatabaseConnection,
        crypto: Arc<SecretBox>,
        settings: SettingsService,
        users: UserService,
        clock: SharedClock,
        public_url: &str,
        fetch: Arc<dyn HttpFetch>,
    ) -> Self {
        let inner = Inner {
            db,
            crypto,
            settings,
            users,
            discovery: Discovery::new(fetch, Arc::clone(&clock)),
            clock,
            redirect_uri: redirect_uri(public_url),
            buttons: providers::ButtonCache::default(),
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn redirect_uri(&self) -> Option<&Url> {
        self.inner.redirect_uri.as_ref()
    }
}

impl Inner {
    fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }
}

fn redirect_uri(public_url: &str) -> Option<Url> {
    let trimmed = public_url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut base = Url::parse(trimmed).ok()?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.join(CALLBACK_PATH).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uri_follows_public_url_and_keeps_its_prefix() {
        let uri = |s: &str| redirect_uri(s).map(|u| u.to_string());

        assert_eq!(
            uri("https://rd.example.com").as_deref(),
            Some("https://rd.example.com/api/oidc/callback")
        );
        assert_eq!(
            uri("https://rd.example.com/").as_deref(),
            Some("https://rd.example.com/api/oidc/callback")
        );
        assert_eq!(
            uri("https://example.com/rustdesk").as_deref(),
            Some("https://example.com/rustdesk/api/oidc/callback")
        );
        assert_eq!(uri(""), None);
        assert_eq!(uri("   "), None);
    }
}
