//! Opaque session tokens (06 §3.2): 256 random bits, base64url without padding. Only the
//! SHA-256 of a token is ever stored or used as a cache key.

use std::fmt;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto::{CryptoError, fill_random};

/// Random bytes per token.
pub const TOKEN_BYTES: usize = 32;
/// Length of the encoded token: `ceil(32 * 4 / 3)` base64 characters, unpadded.
pub const TOKEN_LEN: usize = 43;

/// A freshly issued session token. Handed out exactly once, at creation; zeroed on drop,
/// `Debug` never shows it and there is no `Display` or `Clone`.
pub struct SessionToken(Zeroizing<String>);

impl SessionToken {
    pub(crate) fn generate() -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new([0u8; TOKEN_BYTES]);
        fill_random(bytes.as_mut())?;
        Ok(Self(Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_ref()))))
    }

    /// The token to send to the client (once).
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub(crate) fn hash(&self) -> TokenHash {
        TokenHash::digest(&self.0)
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(<redacted>)")
    }
}

/// Lowercase hex SHA-256 of a token: the `sessions.token_hash` column and the cache key.
/// The token cannot be recovered from it, but it is a lookup key, so it is never logged.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct TokenHash(String);

impl TokenHash {
    /// The hash of a presented token, or `None` when it cannot be one of ours (wrong
    /// length or alphabet): such input never reaches the cache or the database.
    pub fn of_presented(token: &str) -> Option<Self> {
        is_well_formed(token).then(|| Self::digest(token))
    }

    fn digest(token: &str) -> Self {
        Self(format!("{:x}", Sha256::digest(token.as_bytes())))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for TokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenHash(<redacted>)")
    }
}

fn is_well_formed(token: &str) -> bool {
    token.len() == TOKEN_LEN
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn tokens_are_43_url_safe_characters_of_32_random_bytes() {
        let token = SessionToken::generate().unwrap();

        let raw = URL_SAFE_NO_PAD.decode(token.expose()).unwrap();

        assert_eq!(token.expose().len(), TOKEN_LEN);
        assert_eq!(raw.len(), TOKEN_BYTES);
        assert!(is_well_formed(token.expose()));
    }

    #[test]
    fn tokens_do_not_repeat_and_use_the_whole_alphabet() {
        let tokens: Vec<String> = (0..2000)
            .map(|_| SessionToken::generate().unwrap().expose().to_owned())
            .collect();

        let distinct: HashSet<&String> = tokens.iter().collect();
        let symbols: HashSet<char> = tokens.iter().flat_map(|t| t.chars()).collect();

        assert_eq!(distinct.len(), tokens.len());
        // 86 000 symbols drawn from 64: every one appears unless the RNG is broken.
        assert_eq!(symbols.len(), 64, "{symbols:?}");
    }

    #[test]
    fn hash_is_lowercase_hex_sha256_and_never_the_token() {
        let token = SessionToken::generate().unwrap();

        let hash = token.hash();

        assert_eq!(hash.as_str().len(), 64);
        assert!(
            hash.as_str()
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_ne!(hash.as_str(), token.expose());
        assert_eq!(TokenHash::of_presented(token.expose()), Some(hash));
    }

    #[test]
    fn hash_matches_a_known_sha256_vector() {
        // FIPS 180-2 appendix B.1.
        let hash = TokenHash::digest("abc");

        assert_eq!(
            hash.as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn malformed_tokens_have_no_hash() {
        let body = "A".repeat(TOKEN_LEN - 1);
        for bad in [
            String::new(),
            body.clone(),
            format!("{body}AA"),
            format!("{body}="),
            format!("{body}+"),
            format!("{body}/"),
            format!("{body} "),
            format!("{}é", "A".repeat(TOKEN_LEN - 2)),
        ] {
            assert_eq!(TokenHash::of_presented(&bad), None, "{bad:?}");
        }
        assert!(TokenHash::of_presented(&format!("{body}-")).is_some());
        assert!(TokenHash::of_presented(&format!("{body}_")).is_some());
    }

    #[test]
    fn debug_never_shows_token_or_hash() {
        let token = SessionToken::generate().unwrap();
        let hash = token.hash();

        let shown = format!("{token:?} {hash:?}");

        assert!(!shown.contains(token.expose()));
        assert!(!shown.contains(hash.as_str()));
    }
}
