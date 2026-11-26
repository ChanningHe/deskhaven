//! The list is SecLists' top-100k reduced to entries of at least 8 characters, lowercased,
//! byte-order sorted and deduplicated, searched in place by bisection: no index, no startup cost.

use std::cmp::Ordering;

use zeroize::Zeroizing;

use super::Password;

pub const MAX_PASSWORD_LEN: usize = 256;

/// Strictly increasing in byte order (`LC_ALL=C sort -u`).
const COMMON_PASSWORDS: &str = include_str!("../../data/common-passwords.txt");

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyViolation {
    #[error("must be at least {min} characters")]
    TooShort { min: u32 },

    #[error("must be at most {max} characters")]
    TooLong { max: usize },

    #[error("is too common")]
    Common,
}

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
    if contains_line(COMMON_PASSWORDS, &lowered) {
        return Err(PolicyViolation::Common);
    }
    Ok(())
}

/// `lo` and `hi` always sit on line starts (or the end).
fn contains_line(sorted: &str, needle: &str) -> bool {
    let bytes = sorted.as_bytes();
    let (mut lo, mut hi) = (0, bytes.len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let start = bytes[lo..mid]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(lo, |i| lo + i + 1);
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| start + i);
        // Line boundaries are ASCII newlines, hence char boundaries.
        match sorted.get(start..end).map(|line| line.cmp(needle)) {
            Some(Ordering::Equal) => return true,
            Some(Ordering::Less) => lo = end + 1,
            Some(Ordering::Greater) => hi = start,
            None => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_list_is_sorted_deduplicated_lowercase_and_long_enough() {
        let lines: Vec<&str> = COMMON_PASSWORDS.lines().collect();

        assert!(lines.len() > 30_000, "{}", lines.len());
        assert!(COMMON_PASSWORDS.ends_with('\n'));
        for pair in lines.windows(2) {
            assert!(pair[0] < pair[1], "not strictly increasing: {pair:?}");
        }
        for entry in &lines {
            assert_eq!(*entry, entry.trim(), "{entry:?}");
            assert_eq!(*entry, entry.to_lowercase(), "{entry:?}");
            assert!(entry.chars().count() >= 8, "{entry:?}");
        }
    }

    #[test]
    fn every_listed_entry_is_found_and_neighbours_are_not() {
        for entry in COMMON_PASSWORDS.lines() {
            assert!(contains_line(COMMON_PASSWORDS, entry), "{entry:?}");
            let longer = format!("{entry}\u{1}");
            assert!(!contains_line(COMMON_PASSWORDS, &longer), "{longer:?}");
        }
        for absent in [
            "",
            "\n",
            "zzzzzzzzzzzzzzzzzz",
            "        ",
            "vxq9-tundra-lamp",
        ] {
            assert!(!contains_line(COMMON_PASSWORDS, absent), "{absent:?}");
        }
    }

    #[test]
    fn bisection_handles_small_inputs() {
        assert!(!contains_line("", "a"));
        assert!(contains_line("a\n", "a"));
        assert!(contains_line("a", "a"));
        assert!(contains_line("a\nb\nc\n", "a"));
        assert!(contains_line("a\nb\nc\n", "c"));
        assert!(!contains_line("a\nc\n", "b"));
        assert!(!contains_line("ab\nb\n", "a"));
    }
}
