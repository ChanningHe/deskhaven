//! Wire types of the OIDC provider endpoints. The client secret is write-only: requests
//! may carry it, responses only say whether one is set. Timestamps are unix milliseconds.

use deskhaven_core::oidc::{MAX_ICON_LEN, MAX_ISSUER_LEN, NewProvider, Provider, ProviderUpdate};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::extract::present;
use crate::groups::ManagedByView;

const MAX_DISPLAY_NAME: usize = 128;
const MAX_CLIENT_ID: usize = 256;
const MAX_CLIENT_SECRET: usize = 1024;
const MAX_SCOPES: usize = 512;
const MAX_CLAIM: usize = 128;
const MAX_GROUP: usize = 256;

/// An OIDC provider, without its client secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct OidcProviderView {
    pub id: i64,
    /// The client's `op` (`[A-Za-z0-9._-]`, 1-32).
    pub name: String,
    pub display_name: String,
    /// SVG document shown on the client's login button.
    pub icon: Option<String>,
    /// Fixed at creation (identities are keyed by it).
    pub issuer: String,
    pub client_id: String,
    /// `false`: a public client (PKCE only).
    pub has_client_secret: bool,
    /// Space-separated, always including `openid`.
    pub scopes: String,
    /// Unknown identities get a new account.
    pub auto_provision: bool,
    /// ID token claim listing the user's groups.
    pub group_claim: Option<String>,
    /// A `group_claim` value that makes a provisioned account an admin. Only safe when the
    /// claim is controlled solely by the IdP's administrators: never with a multi-tenant IdP
    /// or one where users can change their own groups.
    pub admin_group: Option<String>,
    pub enabled: bool,
    pub managed_by: ManagedByView,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<Provider> for OidcProviderView {
    fn from(p: Provider) -> Self {
        Self {
            id: p.id,
            name: p.name,
            display_name: p.display_name,
            icon: p.icon,
            issuer: p.issuer,
            client_id: p.client_id,
            has_client_secret: p.has_client_secret,
            scopes: p.scopes,
            auto_provision: p.auto_provision,
            group_claim: p.group_claim,
            admin_group: p.admin_group,
            enabled: p.enabled,
            managed_by: p.managed_by.into(),
            created_at: p.created_at.timestamp_millis(),
            updated_at: p.updated_at.timestamp_millis(),
        }
    }
}

/// Every provider and the callback URL to register at the IdPs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct OidcProviderList {
    /// `<public_url>/api/oidc/callback`; `null` without `public_url` (OIDC logins are
    /// refused until it is set).
    pub callback_url: Option<String>,
    /// Ordered by name.
    pub items: Vec<OidcProviderView>,
}

#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewOidcProviderBody {
    #[garde(length(chars, min = 1, max = 32))]
    pub name: String,
    /// Blank: the name.
    #[serde(default)]
    #[garde(length(chars, max = MAX_DISPLAY_NAME))]
    pub display_name: String,
    #[garde(length(max = MAX_ICON_LEN))]
    pub icon: Option<String>,
    /// https (plain http only to a loopback host); cannot change later.
    #[garde(length(max = MAX_ISSUER_LEN))]
    pub issuer: String,
    #[garde(length(chars, min = 1, max = MAX_CLIENT_ID))]
    pub client_id: String,
    /// Write-only; absent or blank: a public client.
    #[garde(length(max = MAX_CLIENT_SECRET))]
    #[schema(format = Password)]
    pub client_secret: Option<String>,
    /// Absent: `openid profile email`.
    #[garde(length(max = MAX_SCOPES))]
    pub scopes: Option<String>,
    #[serde(default)]
    #[garde(skip)]
    pub auto_provision: bool,
    #[garde(length(chars, max = MAX_CLAIM))]
    pub group_claim: Option<String>,
    /// A `group_claim` value that makes a provisioned account an admin. Only safe when the
    /// claim is controlled solely by the IdP's administrators: never with a multi-tenant IdP
    /// or one where users can change their own groups.
    #[garde(length(chars, max = MAX_GROUP))]
    pub admin_group: Option<String>,
    #[serde(default = "enabled_by_default")]
    #[garde(skip)]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

impl From<NewOidcProviderBody> for NewProvider {
    fn from(b: NewOidcProviderBody) -> Self {
        Self {
            name: b.name,
            display_name: b.display_name,
            icon: b.icon,
            issuer: b.issuer,
            client_id: b.client_id,
            client_secret: b.client_secret,
            scopes: b.scopes,
            auto_provision: b.auto_provision,
            group_claim: b.group_claim,
            admin_group: b.admin_group,
            enabled: b.enabled,
        }
    }
}

/// Changes to a provider; absent fields stay as they are, `null` clears a nullable one.
/// The issuer cannot change.
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OidcProviderPatch {
    #[garde(length(chars, min = 1, max = 32))]
    pub name: Option<String>,
    #[garde(length(chars, max = MAX_DISPLAY_NAME))]
    pub display_name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(max = MAX_ICON_LEN))]
    pub icon: Option<Option<String>>,
    #[garde(length(chars, min = 1, max = MAX_CLIENT_ID))]
    pub client_id: Option<String>,
    /// A new secret; `null` makes the provider a public client (an empty string is refused).
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable, format = Password)]
    #[garde(length(max = MAX_CLIENT_SECRET))]
    pub client_secret: Option<Option<String>>,
    #[garde(length(max = MAX_SCOPES))]
    pub scopes: Option<String>,
    #[garde(skip)]
    pub auto_provision: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_CLAIM))]
    pub group_claim: Option<Option<String>>,
    /// `null` removes it. A `group_claim` value that makes a provisioned account an admin. Only safe when the
    /// claim is controlled solely by the IdP's administrators: never with a multi-tenant IdP
    /// or one where users can change their own groups.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    #[garde(length(chars, max = MAX_GROUP))]
    pub admin_group: Option<Option<String>>,
    #[garde(skip)]
    pub enabled: Option<bool>,
}

impl From<OidcProviderPatch> for ProviderUpdate {
    fn from(p: OidcProviderPatch) -> Self {
        Self {
            name: p.name,
            display_name: p.display_name,
            icon: p.icon,
            client_id: p.client_id,
            client_secret: p.client_secret,
            scopes: p.scopes,
            auto_provision: p.auto_provision,
            group_claim: p.group_claim,
            admin_group: p.admin_group,
            enabled: p.enabled,
        }
    }
}
