//! Wire types of the API token endpoints. Timestamps are unix milliseconds (UTC). Nothing
//! here carries a token hash; the secret exists only in [`ApiTokenCreated`].

use deskhaven_core::api_tokens::{ApiTokenEntry, ApiTokenSecret, Scope, TokenCreator};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::RoleView;

/// What a token may do (06 §3.2). Scopes are independent: none implies another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ScopeView {
    /// `rustdesk --deploy`.
    #[serde(rename = "deploy")]
    Deploy,
    /// `rustdesk --assign`.
    #[serde(rename = "assign")]
    Assign,
    /// Read the admin API.
    #[serde(rename = "admin:read")]
    AdminRead,
    /// Write through the admin API.
    #[serde(rename = "admin:write")]
    AdminWrite,
}

impl From<Scope> for ScopeView {
    fn from(scope: Scope) -> Self {
        match scope {
            Scope::Deploy => Self::Deploy,
            Scope::Assign => Self::Assign,
            Scope::AdminRead => Self::AdminRead,
            Scope::AdminWrite => Self::AdminWrite,
        }
    }
}

impl From<ScopeView> for Scope {
    fn from(scope: ScopeView) -> Self {
        match scope {
            ScopeView::Deploy => Self::Deploy,
            ScopeView::Assign => Self::Assign,
            ScopeView::AdminRead => Self::AdminRead,
            ScopeView::AdminWrite => Self::AdminWrite,
        }
    }
}

/// Whether a token can still authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenStatusView {
    Active,
    Revoked,
    Expired,
}

/// The administrator whose authority a token carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TokenCreatorView {
    pub id: i64,
    pub username: String,
    pub role: RoleView,
}

impl From<TokenCreator> for TokenCreatorView {
    fn from(c: TokenCreator) -> Self {
        Self {
            id: c.id,
            username: c.username,
            role: c.role.into(),
        }
    }
}

/// A token without its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApiTokenView {
    pub id: i64,
    pub name: String,
    /// The first characters of the token, to tell tokens apart.
    pub prefix: String,
    pub scopes: Vec<ScopeView>,
    /// `null`: a system token without creator.
    pub created_by: Option<TokenCreatorView>,
    pub created_at: i64,
    /// `null`: never expires.
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub status: TokenStatusView,
}

impl ApiTokenView {
    /// `now_ms` decides between `active` and `expired`.
    pub fn of(entry: ApiTokenEntry, now_ms: i64) -> Self {
        let t = entry.token;
        let expires_at = t.expires_at.map(|at| at.timestamp_millis());
        let status = match (t.revoked_at, expires_at) {
            (Some(_), _) => TokenStatusView::Revoked,
            (None, Some(at)) if at <= now_ms => TokenStatusView::Expired,
            _ => TokenStatusView::Active,
        };
        Self {
            id: t.id,
            name: t.name,
            prefix: t.token_prefix,
            scopes: t.scopes.into_iter().map(ScopeView::from).collect(),
            created_by: entry.creator.map(Into::into),
            created_at: t.created_at.timestamp_millis(),
            expires_at,
            last_used_at: t.last_used_at.map(|at| at.timestamp_millis()),
            revoked_at: t.revoked_at.map(|at| at.timestamp_millis()),
            status,
        }
    }
}

/// A token's secret as shown once. Serializes as the plain string, but `Debug` prints
/// `<redacted>`, so a logged response or error never carries it.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ShownToken(String);

impl ShownToken {
    pub fn new(secret: &ApiTokenSecret) -> Self {
        Self(secret.expose().to_owned())
    }
}

impl std::fmt::Debug for ShownToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A created token with its secret, shown only here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ApiTokenCreated {
    pub token: ApiTokenView,
    /// The token itself (`dhk_...`); never returned again.
    #[schema(value_type = String)]
    pub secret: ShownToken,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewApiTokenBody {
    /// 1-64 characters.
    #[garde(length(chars, min = 1, max = 64))]
    pub name: String,
    /// At least one.
    #[garde(length(min = 1, max = 4))]
    pub scopes: Vec<ScopeView>,
    /// Unix milliseconds, in the future and at most ten years away; absent or `null`: never
    /// expires.
    #[serde(default)]
    #[garde(skip)]
    pub expires_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn scopes_use_their_stored_names() {
        let names = serde_json::to_value(Scope::ALL.map(ScopeView::from)).unwrap();

        assert_eq!(
            names,
            serde_json::json!(["deploy", "assign", "admin:read", "admin:write"])
        );
    }

    #[test]
    fn shown_tokens_never_debug_print() {
        let created = ShownToken("dhk_secret".to_owned());

        assert_eq!(format!("{created:?}"), "<redacted>");
        assert_eq!(serde_json::to_value(&created).unwrap(), "dhk_secret");
    }
}
