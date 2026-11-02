//! Query parameters of the OIDC endpoints, form-urlencoded: the client builds the poll URL
//! with `Url::parse_with_params` (its uuid is base64, so `+ / =` arrive encoded), and IdPs
//! encode `code` / `state` likewise.

use url::form_urlencoded;

/// Longest accepted parameter value (codes and states are 43 characters here; IdP codes
/// and error names are short).
const MAX_VALUE_LEN: usize = 2048;

/// The decoded value of the first parameter named `key`; `None` when missing, empty or
/// longer than [`MAX_VALUE_LEN`].
pub(super) fn param(query: Option<&str>, key: &str) -> Option<String> {
    form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty() && v.len() <= MAX_VALUE_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_percent_decoded_and_bounded() {
        let query = Some("code=abc&id=123&uuid=aGVs%2BbG8%2F%3D&empty=&code=second");

        assert_eq!(param(query, "code").as_deref(), Some("abc"));
        assert_eq!(param(query, "uuid").as_deref(), Some("aGVs+bG8/="));
        assert_eq!(param(query, "empty"), None);
        assert_eq!(param(query, "missing"), None);
        assert_eq!(param(None, "code"), None);
        let long = format!("code={}", "x".repeat(MAX_VALUE_LEN + 1));
        assert_eq!(param(Some(&long), "code"), None);
    }
}
