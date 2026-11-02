//! Password policy (06 §3.1): `auth.password.min_length` ..= [`MAX_PASSWORD_LEN`] characters,
//! not on the embedded common-password list. Any Unicode is allowed.
//!
//! The list (`data/common-passwords.txt`, provenance and MIT license in
//! `data/common-passwords.LICENSE`) is SecLists' top-100k list reduced to entries of at least
//! 8 characters (the lowest allowed minimum length), lowercased and deduplicated.

use std::collections::HashSet;
use std::sync::LazyLock;

use zeroize::Zeroizing;

use super::Password;

/// Longest accepted password, in characters.
pub const MAX_PASSWORD_LEN: usize = 256;

static COMMON_PASSWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("../../data/common-passwords.txt")
        .lines()
        .collect()
});

/// Why a password was rejected. Stable for API error mapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyViolation {
    #[error("must be at least {min} characters")]
    TooShort { min: u32 },

    #[error("must be at most {max} characters")]
    TooLong { max: usize },

    #[error("is too common")]
    Common,
}

/// Checks `password` against the policy with the given minimum length (in characters).
pub fn check_password_policy(password: &Password, min_length: u32) -> Result<(), PolicyViolation> {
    let len = password.char_len();
    if len < usize::try_from(min_length).unwrap_or(usize::MAX) {
        return Err(PolicyViolation::TooShort { min: min_length });
    }
    if len > MAX_PASSWORD_LEN {
        return Err(PolicyViolation::TooLong {
            max: MAX_PASSWORD_LEN,
        });
    }
    let lowered = Zeroizing::new(password.expose().to_lowercase());
    if COMMON_PASSWORDS.contains(lowered.as_str()) {
        return Err(PolicyViolation::Common);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_list_is_lowercase_trimmed_and_long_enough() {
        let list = &*COMMON_PASSWORDS;

        assert!(list.len() > 30_000, "{}", list.len());
        for entry in list {
            assert_eq!(*entry, entry.trim(), "{entry:?}");
            assert_eq!(*entry, entry.to_lowercase(), "{entry:?}");
            assert!(entry.chars().count() >= 8, "{entry:?}");
        }
    }
}
