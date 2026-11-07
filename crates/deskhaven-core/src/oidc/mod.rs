//! OpenID Connect login (06 §3.4): providers, and the authorization-code flow with PKCE that
//! the official client drives by polling (03 §3.5–3.6).
//!
//! # Flow (client)
//!
//! 1. [`OidcService::begin_client`] (`POST /api/oidc/auth`): one `oidc_flows` row holding
//!    SHA-256 of a 256-bit `state`, SHA-256 of a 256-bit client polling `code`, the PKCE S256
//!    verifier sealed with [`SecretBox`] under `oidc_flows.pkce_verifier_enc:<state_hash>`
//!    (the row id is unknown before the insert; the state hash is unique per row), the
//!    `nonce`, the device the session will be bound to (`context`) and `expires_at`
//!    ([`FLOW_TTL`]). Returns the polling code and the IdP's authorization URL.
//! 2. The IdP redirects the browser to `GET <public_url>/api/oidc/callback`, which only
//!    shows [`OidcService::preview_callback`]: which device (RustDesk id, host name, OS from
//!    the flow) is about to be signed in, with a button. Nothing is consumed (see `preview`:
//!    cross-device consent phishing). The confirming `POST` runs
//!    [`OidcService::complete_callback`]: it claims the
//!    flow by its state (`UPDATE … SET consumed_at WHERE consumed_at IS NULL AND
//!    expires_at > now`, so a state works once), exchanges the code with the verifier,
//!    verifies the ID token (signature against the provider's JWKS, `iss`, `aud`, `exp`,
//!    `nonce`), resolves the identity (`identity`), and records the outcome on the row
//!    (`completed_user_id` plus the account's credential stamp in `context`, or `error`).
//! 3. [`OidcService::poll_client`] (`GET /api/oidc/auth-query`): pending until the callback
//!    finished; then, for the device the flow was started from only, hands the account out
//!    exactly once by deleting the row (`DELETE … WHERE completed_user_id IS NOT NULL`); the
//!    caller mints the session from the stored stamp.
//!
//! # Flow (console)
//!
//! [`OidcService::begin_console`] stores a `flow = console` row (no polling code) whose
//! context holds SHA-256 of a 256-bit browser binding the console keeps in a cookie. The
//! callback page sends the browser on to the console, which completes the flow with
//! [`OidcService::complete_console`]: it requires that binding (constant-time compare)
//! before it claims the row, so the browser that started the login is the one that
//! finishes it. That makes the client flow's confirmation step unnecessary here: a
//! forwarded authorization URL or a planted callback link (login CSRF) lands in a browser
//! without the binding and does nothing. The session cookie is the caller's business; the
//! row is deleted once the flow has an outcome.
//!
//! # Identity (06 §3.4)
//!
//! The identity key is `(provider_id, sub)` (`user_identities`). It stands for
//! `(issuer, sub)` because a provider's issuer can never change after creation (the update
//! API has no issuer field); a different issuer is a different provider. Each identity
//! records how it was linked (`linked_via`): `provisioned` (the IdP login created the
//! account), `explicit` (the signed-in account holder linked it, `link`) or `email`. An
//! unknown identity is linked to an existing account by email only when the ID token says
//! `email_verified: true` (a missing claim counts as false), the account's own address is
//! verified, its role is within `auth.oidc.link_email_max_role`, it has no TOTP and it is
//! not an LDAP account; an `email` identity is re-checked against the same conditions at
//! every login (see `identity`). Otherwise, when the email belongs to an account, the login
//! is refused (the holder signs in with the account and links the identity explicitly).
//! With no such account and `auto_provision` on, a new `oidc` account is created. Only
//! IdP-verified addresses are ever stored. Accounts are never matched by username. Disabled
//! and unverified accounts are refused. Accepted: the refusal versus provisioning difference
//! tells an IdP user whether an address has a deskhaven account (the clearer experience for
//! its real owner).
//!
//! OIDC logins do not ask for the account's local second factor, and `auth.tfa.enforce`
//! constrains password logins only: the IdP authenticates, and multi-factor policy belongs
//! there. That is why accounts with TOTP are never linked by email, only explicitly by
//! their holder.
//!
//! # Callback URL
//!
//! `<public_url>/api/oidc/callback`, registered at the IdP. With a path prefix in
//! `public_url` (`https://example.com/rustdesk`), the reverse proxy must strip the prefix
//! before forwarding (deskhaven serves `/api/oidc/callback`).
//!
//! # Outbound HTTP
//!
//! See `http`: https only (plain http only to a loopback issuer, and then only to loopback
//! endpoints), no link-local or cloud-metadata addresses (`resolver`, also as IP literals),
//! no redirects, no proxy, timeouts and a response size cap on every request. A failed
//! discovery is remembered for 30 s per provider (`discovery`), so an IdP outage does not
//! make every login attempt wait for its own timeout. Logs name the provider and the kind
//! of failure only, never upstream bodies or claim values.

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
pub use poll::{ClientLogin, PollOutcome};
pub use preview::{CallbackPreview, DeviceSummary, LinkPreview, PreviewTarget};
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

/// Lifetime of a flow, from `/api/oidc/auth` to the last poll. The client gives up after 3
/// minutes; kept short so a phished authorization URL goes stale quickly.
pub const FLOW_TTL: Duration = Duration::from_secs(5 * 60);
/// Path of the callback, below `public_url`.
pub const CALLBACK_PATH: &str = "api/oidc/callback";

/// OIDC providers and logins. Cheap to clone (shared state).
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
    /// `<public_url>/api/oidc/callback`; `None` without `public_url` (OIDC unavailable).
    redirect_uri: Option<Url>,
    discovery: Discovery,
    buttons: providers::ButtonCache,
}

impl OidcService {
    /// `public_url` is the bootstrap value (validated: http(s), host, no query); blank means
    /// unset, which leaves providers manageable but every login refused
    /// ([`OidcError::NotConfigured`]) and no login buttons. `fetch` carries every request to
    /// the IdPs (production: [`ReqwestFetch`]).
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

    /// The fixed callback URL registered at the IdPs, when `public_url` is set.
    pub fn redirect_uri(&self) -> Option<&Url> {
        self.inner.redirect_uri.as_ref()
    }
}

impl Inner {
    fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }
}

/// `<public_url>/api/oidc/callback`, keeping a path prefix of `public_url`.
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
