//! What deskhaven takes from a verified ID token.

use openidconnect::core::CoreGenderClaim;
use openidconnect::{AdditionalClaims, IdTokenClaims};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Longest accepted `sub` (OIDC Core §2: at most 255 ASCII characters).
const MAX_SUBJECT_LEN: usize = 255;
/// Group claim values beyond this many are ignored.
const MAX_GROUPS: usize = 256;
/// Group names longer than this are ignored (deskhaven group names are at most 64).
const MAX_GROUP_LEN: usize = 256;

/// The claims outside the standard set (for the configured group claim).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct ExtraClaims(Map<String, Value>);

impl AdditionalClaims for ExtraClaims {}

pub(crate) type Claims = IdTokenClaims<ExtraClaims, CoreGenderClaim>;

/// The identity an ID token asserts, after verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Asserted {
    pub(crate) subject: String,
    pub(crate) email: Option<String>,
    /// `None` when the claim is missing: treated as unverified (06 §3.4).
    pub(crate) email_verified: Option<bool>,
    pub(crate) preferred_username: Option<String>,
    pub(crate) name: Option<String>,
    /// Values of the provider's group claim, if configured.
    pub(crate) groups: Vec<String>,
}

impl Asserted {
    /// `None` when `sub` is empty or longer than OIDC allows.
    pub(crate) fn from_claims(claims: &Claims, group_claim: Option<&str>) -> Option<Self> {
        let subject = claims.subject().as_str();
        if subject.is_empty() || subject.len() > MAX_SUBJECT_LEN {
            return None;
        }
        Some(Self {
            subject: subject.to_owned(),
            email: claims.email().map(|e| e.as_str().to_owned()),
            email_verified: claims.email_verified(),
            preferred_username: claims.preferred_username().map(|u| u.as_str().to_owned()),
            name: claims
                .name()
                .and_then(|n| n.get(None))
                .map(|n| n.as_str().to_owned()),
            groups: group_claim
                .and_then(|claim| claims.additional_claims().0.get(claim))
                .map(group_values)
                .unwrap_or_default(),
        })
    }

    /// The email, only when the IdP asserts it verified.
    pub(crate) fn verified_email(&self) -> Option<&str> {
        match self.email_verified {
            Some(true) => self.email.as_deref(),
            _ => None,
        }
    }
}

/// A string or an array of strings; anything else contributes nothing.
fn group_values(value: &Value) -> Vec<String> {
    let usable = |s: &&str| !s.is_empty() && s.len() <= MAX_GROUP_LEN;
    match value {
        Value::String(one) => [one.as_str()]
            .into_iter()
            .filter(usable)
            .map(str::to_owned)
            .collect(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .filter(usable)
            .take(MAX_GROUPS)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn group_claims_take_strings_and_string_arrays_only() {
        assert_eq!(group_values(&json!("ops")), vec!["ops"]);
        assert_eq!(
            group_values(&json!(["ops", 1, "dev", "", null])),
            vec!["ops", "dev"]
        );
        assert!(group_values(&json!({"ops": true})).is_empty());
        assert!(group_values(&json!("x".repeat(MAX_GROUP_LEN + 1))).is_empty());
        let many: Vec<String> = (0..MAX_GROUPS + 10).map(|i| format!("g{i}")).collect();
        assert_eq!(group_values(&json!(many)).len(), MAX_GROUPS);
    }
}
