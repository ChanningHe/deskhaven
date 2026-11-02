//! Recovery codes (06 §3.3): [`RECOVERY_CODE_COUNT`] single-use codes of
//! [`RECOVERY_CODE_CHARS`] lowercase base32 characters (50 random bits each), shown as
//! `xxxxx-xxxxx`. Stored as `SecretBox::mac` of the normalized code under the context
//! `recovery_codes.code_hash:<user id>`, so a database dump cannot be brute-forced offline
//! and a hash cannot be moved to another user. A presented code is looked up under every
//! master key the `SecretBox` holds (`mac_candidates`), so a key rotation does not void it.
//!
//! Input is normalized before hashing: separators (`-`, spaces) dropped, ASCII lower-cased.
//! Anything else that is not a well-formed code is rejected without a database lookup.

use std::fmt;

use zeroize::Zeroizing;

use crate::crypto::{CryptoError, SecretBox, fill_random};

/// Codes issued per enrolment / regeneration.
pub const RECOVERY_CODE_COUNT: usize = 10;
/// Characters of a code, separators excluded.
pub const RECOVERY_CODE_CHARS: usize = 10;
/// RFC 4648 base32, lower case: 32 symbols, so `byte % 32` is uniform.
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const GROUP: usize = RECOVERY_CODE_CHARS / 2;
const SEPARATOR: char = '-';

/// Freshly generated codes, to show their owner once. `Debug` never shows them.
pub struct RecoveryCodes(Vec<Zeroizing<String>>);

impl RecoveryCodes {
    /// The codes as shown to the user (`xxxxx-xxxxx`).
    pub fn codes(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|code| code.as_str())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for RecoveryCodes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RecoveryCodes({} codes, <redacted>)", self.0.len())
    }
}

/// [`RECOVERY_CODE_COUNT`] new codes and, for each, its stored hash for `user_id`.
pub(crate) fn generate(
    crypto: &SecretBox,
    user_id: i64,
) -> Result<(RecoveryCodes, Vec<String>), CryptoError> {
    let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);
    let mut hashes = Vec::with_capacity(RECOVERY_CODE_COUNT);
    while codes.len() < RECOVERY_CODE_COUNT {
        let code = random_code()?;
        let hash = hash(crypto, user_id, &code);
        // A repeat (2^-50 per pair) would violate UNIQUE(user_id, code_hash).
        if hashes.contains(&hash) {
            continue;
        }
        codes.push(display(&code));
        hashes.push(hash);
    }
    Ok((RecoveryCodes(codes), hashes))
}

/// The stored hash of a normalized code (under the primary master key).
pub(crate) fn hash(crypto: &SecretBox, user_id: i64, normalized: &str) -> String {
    crypto.mac(&context(user_id), normalized.as_bytes())
}

/// Every hash a stored code may have: under the primary key and under each retired key
/// still held, so codes issued before a master-key rotation keep working.
pub(crate) fn candidates(crypto: &SecretBox, user_id: i64, normalized: &str) -> Vec<String> {
    crypto.mac_candidates(&context(user_id), normalized.as_bytes())
}

fn context(user_id: i64) -> String {
    format!("recovery_codes.code_hash:{user_id}")
}

/// The canonical form of user input, or `None` when it cannot be a recovery code.
pub(crate) fn normalize(input: &str) -> Option<Zeroizing<String>> {
    let code: Zeroizing<String> = Zeroizing::new(
        input
            .chars()
            .filter(|c| *c != SEPARATOR && *c != ' ')
            .map(|c| c.to_ascii_lowercase())
            .collect(),
    );
    let well_formed =
        code.len() == RECOVERY_CODE_CHARS && code.bytes().all(|b| ALPHABET.contains(&b));
    well_formed.then_some(code)
}

fn random_code() -> Result<Zeroizing<String>, CryptoError> {
    let mut bytes = Zeroizing::new([0u8; RECOVERY_CODE_CHARS]);
    fill_random(bytes.as_mut())?;
    let code = bytes
        .iter()
        .map(|b| char::from(ALPHABET[usize::from(b % 32)]))
        .collect();
    Ok(Zeroizing::new(code))
}

fn display(code: &str) -> Zeroizing<String> {
    let (head, tail) = code.split_at(GROUP);
    Zeroizing::new(format!("{head}{SEPARATOR}{tail}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::crypto::MasterKey;

    fn crypto() -> SecretBox {
        SecretBox::new(&MasterKey::from_bytes(&[7; 32]))
    }

    #[test]
    fn generates_ten_distinct_well_formed_codes_with_matching_hashes() {
        let crypto = crypto();

        let (codes, hashes) = generate(&crypto, 42).unwrap();

        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        let distinct: HashSet<&str> = codes.codes().collect();
        assert_eq!(distinct.len(), RECOVERY_CODE_COUNT);
        for (code, stored) in codes.codes().zip(&hashes) {
            assert_eq!(code.len(), RECOVERY_CODE_CHARS + 1);
            assert_eq!(code.as_bytes()[GROUP], b'-');
            let normalized = normalize(code).unwrap();
            assert_eq!(&hash(&crypto, 42, &normalized), stored);
        }
    }

    #[test]
    fn normalization_ignores_separators_and_case_only() {
        let canonical = normalize("abcde-fgh23").unwrap();
        assert_eq!(canonical.as_str(), "abcdefgh23");
        for same in [
            "abcdefgh23",
            "ABCDE-FGH23",
            " abcde fgh23 ",
            "ab-cd-ef-gh-23",
        ] {
            assert_eq!(
                normalize(same).as_deref().map(String::as_str),
                Some("abcdefgh23"),
                "{same:?}"
            );
        }
        for bad in [
            "",
            "abcde-fgh2",
            "abcde-fgh234",
            "abcde-fgh01",
            "abcde_fgh23",
            "abcdé-fgh23",
            "123456",
        ] {
            assert!(normalize(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn hashes_are_bound_to_the_user() {
        let crypto = crypto();
        assert_ne!(
            hash(&crypto, 1, "abcdefgh23"),
            hash(&crypto, 2, "abcdefgh23")
        );
    }

    #[test]
    fn debug_never_shows_codes() {
        let (codes, _) = generate(&crypto(), 1).unwrap();
        let shown = format!("{codes:?}");
        assert!(codes.codes().all(|code| !shown.contains(code)), "{shown}");
    }

    #[test]
    fn every_symbol_appears() {
        let symbols: HashSet<char> = (0..200)
            .flat_map(|_| random_code().unwrap().chars().collect::<Vec<_>>())
            .collect();
        // 2000 draws from 32 symbols: all appear unless the mapping is broken.
        assert_eq!(symbols.len(), 32);
    }
}
