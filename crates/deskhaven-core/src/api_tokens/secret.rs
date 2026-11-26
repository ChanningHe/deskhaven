use std::fmt;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::URL_SAFE_NO_PAD;
use zeroize::Zeroizing;

use crate::crypto::{CryptoError, fill_random};
use crate::sessions::{TOKEN_BYTES, TOKEN_LEN, TokenHash};

/// Lets scanners and humans tell what it is.
pub const API_TOKEN_PREFIX: &str = "dhk_";
pub const DISPLAY_PREFIX_LEN: usize = API_TOKEN_PREFIX.len() + 8;

/// Handed out once; zeroed on drop, hidden from `Debug`, no `Display` or `Clone`.
pub struct ApiTokenSecret(Zeroizing<String>);

impl ApiTokenSecret {
    pub(super) fn generate() -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new([0u8; TOKEN_BYTES]);
        fill_random(bytes.as_mut())?;
        let body = Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_ref()));
        Ok(Self(Zeroizing::new(format!("{API_TOKEN_PREFIX}{}", *body))))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(super) fn hash(&self) -> TokenHash {
        TokenHash::digest(&self.0)
    }

    pub(super) fn display_prefix(&self) -> String {
        self.0.chars().take(DISPLAY_PREFIX_LEN).collect()
    }
}

impl fmt::Debug for ApiTokenSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiTokenSecret(<redacted>)")
    }
}

/// Input that cannot be one of ours never reaches the database.
pub(super) fn presented_hash(token: &str) -> Option<TokenHash> {
    let body = token.strip_prefix(API_TOKEN_PREFIX)?;
    let well_formed = body.len() == TOKEN_LEN
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    well_formed.then(|| TokenHash::digest(token))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn tokens_are_the_prefix_and_43_url_safe_characters_of_32_random_bytes() {
        let secret = ApiTokenSecret::generate().unwrap();

        let body = secret.expose().strip_prefix(API_TOKEN_PREFIX).unwrap();

        assert_eq!(URL_SAFE_NO_PAD.decode(body).unwrap().len(), TOKEN_BYTES);
        assert_eq!(presented_hash(secret.expose()), Some(secret.hash()));
        assert!(secret.expose().starts_with(&secret.display_prefix()));
        assert_eq!(secret.display_prefix().len(), DISPLAY_PREFIX_LEN);
    }

    #[test]
    fn tokens_do_not_repeat() {
        let tokens: HashSet<String> = (0..1000)
            .map(|_| ApiTokenSecret::generate().unwrap().expose().to_owned())
            .collect();

        assert_eq!(tokens.len(), 1000);
    }

    #[test]
    fn malformed_tokens_have_no_hash() {
        let body = "A".repeat(TOKEN_LEN);
        for bad in [
            String::new(),
            body.clone(),
            format!("dhx_{body}"),
            format!("DHK_{body}"),
            format!("{API_TOKEN_PREFIX}{body}A"),
            format!("{API_TOKEN_PREFIX}{}", &body[1..]),
            format!("{API_TOKEN_PREFIX}{}=", &body[1..]),
            format!("{API_TOKEN_PREFIX}{}+", &body[1..]),
        ] {
            assert_eq!(presented_hash(&bad), None, "{bad:?}");
        }
    }

    #[test]
    fn debug_never_shows_the_token() {
        let secret = ApiTokenSecret::generate().unwrap();

        assert!(!format!("{secret:?}").contains(secret.expose()));
    }
}
