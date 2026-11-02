//! Provider types and input validation (04 §3 `oidc_providers`).

use std::fmt;

use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::oidc_provider;
use deskhaven_db::time::TimestampMs;

use super::super::error::ProviderError;
use super::super::url_policy::check_issuer;
use crate::text;

/// Longest provider name (the client's `op`).
pub const MAX_NAME_LEN: usize = 32;
/// Largest icon (an SVG document the client renders inline, `login.dart:79-82`).
pub const MAX_ICON_LEN: usize = 16 * 1024;
const MAX_DISPLAY_NAME_LEN: usize = 128;
const MAX_CLIENT_ID_LEN: usize = 256;
const MAX_CLIENT_SECRET_LEN: usize = 1024;
const MAX_SCOPES_LEN: usize = 512;
const MAX_CLAIM_NAME_LEN: usize = 128;
const MAX_GROUP_NAME_LEN: usize = 256;
pub(crate) const DEFAULT_SCOPES: &str = "openid profile email";
const OPENID: &str = "openid";

/// A provider as seen outside the service: never the client secret, only whether one is set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub icon: Option<String>,
    pub issuer: String,
    pub client_id: String,
    /// `false`: a public client (PKCE only).
    pub has_client_secret: bool,
    /// Space-separated, always including `openid`.
    pub scopes: String,
    pub auto_provision: bool,
    /// Claim of the ID token listing the user's groups (string or array of strings).
    pub group_claim: Option<String>,
    /// A value of `group_claim` that makes a provisioned account an admin.
    pub admin_group: Option<String>,
    pub enabled: bool,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl From<oidc_provider::Model> for Provider {
    fn from(m: oidc_provider::Model) -> Self {
        Self {
            id: m.id,
            name: m.name,
            display_name: m.display_name,
            icon: m.icon,
            issuer: m.issuer,
            client_id: m.client_id,
            has_client_secret: m.client_secret_enc.is_some(),
            scopes: m.scopes,
            auto_provision: m.auto_provision,
            group_claim: m.group_claim,
            admin_group: m.admin_group,
            enabled: m.enabled,
            managed_by: m.managed_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Input of [`OidcService::create_provider`](super::super::OidcService::create_provider).
/// `Debug` never shows the client secret.
#[derive(Clone)]
pub struct NewProvider {
    /// `[A-Za-z0-9._-]`, 1–32 characters; the client's `op`.
    pub name: String,
    /// Blank: the name.
    pub display_name: String,
    pub icon: Option<String>,
    /// https (or http on a loopback host); never changes after creation.
    pub issuer: String,
    pub client_id: String,
    /// `None`: a public client.
    pub client_secret: Option<String>,
    /// `None`: `openid profile email`.
    pub scopes: Option<String>,
    pub auto_provision: bool,
    pub group_claim: Option<String>,
    pub admin_group: Option<String>,
    pub enabled: bool,
}

impl NewProvider {
    /// An enabled public client with the default scopes, no icon, no provisioning.
    pub fn new(
        name: impl Into<String>,
        issuer: impl Into<String>,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            display_name: String::new(),
            icon: None,
            issuer: issuer.into(),
            client_id: client_id.into(),
            client_secret: None,
            scopes: None,
            auto_provision: false,
            group_claim: None,
            admin_group: None,
            enabled: true,
        }
    }
}

impl fmt::Debug for NewProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewProvider")
            .field("name", &self.name)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("has_client_secret", &self.client_secret.is_some())
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

/// Fields to change; `None` leaves a field as it is, `Some(None)` clears an optional one.
/// The issuer cannot change (identities are keyed by it). `Debug` never shows the secret.
#[derive(Clone, Default)]
pub struct ProviderUpdate {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub icon: Option<Option<String>>,
    pub client_id: Option<String>,
    /// `Some(None)` makes it a public client.
    pub client_secret: Option<Option<String>>,
    pub scopes: Option<String>,
    pub auto_provision: Option<bool>,
    pub group_claim: Option<Option<String>>,
    pub admin_group: Option<Option<String>>,
    pub enabled: Option<bool>,
}

impl fmt::Debug for ProviderUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderUpdate")
            .field("name", &self.name)
            .field("client_id", &self.client_id)
            .field("changes_client_secret", &self.client_secret.is_some())
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

/// A login button of `/api/login-options`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginButton {
    pub name: String,
    pub icon: Option<String>,
}

fn invalid(field: &'static str, reason: impl Into<String>) -> ProviderError {
    ProviderError::Invalid {
        field,
        reason: reason.into(),
    }
}

pub(crate) fn name(raw: &str) -> Result<String, ProviderError> {
    let ok = !raw.is_empty()
        && raw.len() <= MAX_NAME_LEN
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    ok.then(|| raw.to_owned()).ok_or_else(|| {
        invalid(
            "name",
            format!("must be 1-{MAX_NAME_LEN} characters of A-Z a-z 0-9 . _ -"),
        )
    })
}

/// Trimmed; blank falls back to `name`.
pub(crate) fn display_name(raw: &str, name: &str) -> Result<String, ProviderError> {
    let trimmed = raw.trim();
    let value = if trimmed.is_empty() { name } else { trimmed };
    plain("display_name", value, MAX_DISPLAY_NAME_LEN, false)
}

/// Blank is no icon.
pub(crate) fn icon(raw: Option<&str>) -> Result<Option<String>, ProviderError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(svg) if svg.len() > MAX_ICON_LEN => Err(invalid(
            "icon",
            format!("must be at most {MAX_ICON_LEN} bytes"),
        )),
        Some(svg) => plain("icon", svg, MAX_ICON_LEN, true).map(Some),
    }
}

pub(crate) fn issuer(raw: &str) -> Result<String, ProviderError> {
    let trimmed = raw.trim();
    check_issuer(trimmed).map_err(|reason| invalid("issuer", reason))?;
    // Kept as entered: the ID token's `iss` must equal it exactly (trailing slash included).
    Ok(trimmed.to_owned())
}

pub(crate) fn client_id(raw: &str) -> Result<String, ProviderError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid("client_id", "must not be empty"));
    }
    plain("client_id", trimmed, MAX_CLIENT_ID_LEN, false)
}

/// Blank is no secret (public client). Not trimmed: secrets are opaque.
pub(crate) fn client_secret(raw: Option<&str>) -> Result<Option<String>, ProviderError> {
    match raw.filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(secret)
            if secret.len() > MAX_CLIENT_SECRET_LEN || secret.chars().any(char::is_control) =>
        {
            Err(invalid(
                "client_secret",
                format!("must be at most {MAX_CLIENT_SECRET_LEN} bytes without control characters"),
            ))
        }
        Some(secret) => Ok(Some(secret.to_owned())),
    }
}

/// Space-separated RFC 6749 scope tokens, deduplicated, `openid` required.
pub(crate) fn scopes(raw: Option<&str>) -> Result<String, ProviderError> {
    let raw = raw.unwrap_or(DEFAULT_SCOPES);
    if raw.len() > MAX_SCOPES_LEN {
        return Err(invalid(
            "scopes",
            format!("must be at most {MAX_SCOPES_LEN} characters"),
        ));
    }
    let mut tokens: Vec<&str> = Vec::new();
    for token in raw.split_whitespace() {
        let valid = token
            .bytes()
            .all(|b| b == 0x21 || (0x23..=0x5B).contains(&b) || (0x5D..=0x7E).contains(&b));
        if !valid {
            return Err(invalid(
                "scopes",
                "contains a character not allowed in a scope",
            ));
        }
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    if !tokens.contains(&OPENID) {
        return Err(invalid("scopes", "must include openid"));
    }
    Ok(tokens.join(" "))
}

/// A claim name: no whitespace or control characters; blank is none.
pub(crate) fn group_claim(raw: Option<&str>) -> Result<Option<String>, ProviderError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(claim)
            if claim.len() > MAX_CLAIM_NAME_LEN || claim.chars().any(char::is_whitespace) =>
        {
            Err(invalid(
                "group_claim",
                format!("must be at most {MAX_CLAIM_NAME_LEN} characters without whitespace"),
            ))
        }
        Some(claim) => plain("group_claim", claim, MAX_CLAIM_NAME_LEN, false).map(Some),
    }
}

/// Blank is none.
pub(crate) fn admin_group(raw: Option<&str>) -> Result<Option<String>, ProviderError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(group) => plain("admin_group", group, MAX_GROUP_NAME_LEN, false).map(Some),
    }
}

fn plain(
    field: &'static str,
    value: &str,
    max: usize,
    multiline: bool,
) -> Result<String, ProviderError> {
    match text::problem(value, max, multiline) {
        Some(reason) => Err(invalid(field, reason)),
        None => Ok(value.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_short_ascii_tokens() {
        assert_eq!(name("google").unwrap(), "google");
        assert_eq!(name("Corp.SSO_1-a").unwrap(), "Corp.SSO_1-a");
        for bad in ["", "a b", "ünï", "x/y", &"a".repeat(MAX_NAME_LEN + 1)] {
            assert!(name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn scopes_require_openid_and_are_deduplicated() {
        assert_eq!(scopes(None).unwrap(), DEFAULT_SCOPES);
        assert_eq!(
            scopes(Some(" email  openid email groups ")).unwrap(),
            "email openid groups"
        );
        assert!(scopes(Some("profile email")).is_err());
        assert!(scopes(Some("openid \"quoted\"")).is_err());
        assert!(scopes(Some("openid back\\slash")).is_err());
    }

    #[test]
    fn optional_fields_treat_blank_as_none() {
        assert_eq!(icon(Some("  ")).unwrap(), None);
        assert_eq!(client_secret(Some("")).unwrap(), None);
        assert_eq!(group_claim(Some(" ")).unwrap(), None);
        assert_eq!(admin_group(None).unwrap(), None);
        assert!(group_claim(Some("a b")).is_err());
        assert!(icon(Some(&"x".repeat(MAX_ICON_LEN + 1))).is_err());
        assert!(client_secret(Some("a\nb")).is_err());
    }

    #[test]
    fn display_name_falls_back_to_the_name() {
        assert_eq!(display_name("  ", "google").unwrap(), "google");
        assert_eq!(display_name(" Corp SSO ", "corp").unwrap(), "Corp SSO");
    }

    #[test]
    fn debug_hides_the_client_secret() {
        let mut new = NewProvider::new("corp", "https://idp.example.com", "deskhaven");
        new.client_secret = Some("s3cret-value".to_owned());
        let update = ProviderUpdate {
            client_secret: Some(Some("s3cret-value".to_owned())),
            ..ProviderUpdate::default()
        };

        let shown = format!("{new:?} {update:?}");

        assert!(!shown.contains("s3cret-value"), "{shown}");
    }
}
