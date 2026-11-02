//! Usernames of provisioned accounts: the same charset and length as every username
//! (`[A-Za-z0-9._@-]`, 1–64, `users::username_norm`), so they also key the login lockout
//! like any other.
//!
//! The base is the first of `preferred_username`, `email`, `sub` that keeps at least one
//! character after dropping the others (e.g. `Zoë Smith` -> `ZoSmith`), else
//! [`FALLBACK`]. A taken name gets `-2`, `-3`, … ([`MAX_SUFFIX`]); the base is shortened to
//! keep the result within 64 characters. An existing account is never reused because its
//! name matches: the suffix makes a new one.

use super::claims::Asserted;
use crate::users::{USERNAME_MAX_LEN, is_username_char, username_norm};

/// Base when no claim has a usable character.
pub(crate) const FALLBACK: &str = "oidc-user";
/// Highest numeric suffix tried before provisioning gives up.
pub(crate) const MAX_SUFFIX: u32 = 50;

/// The usable characters of `raw`, at most 64.
fn sanitize(raw: &str) -> String {
    raw.chars()
        .filter(|c| is_username_char(*c))
        .take(USERNAME_MAX_LEN)
        .collect()
}

pub(crate) fn base(asserted: &Asserted) -> String {
    [
        asserted.preferred_username.as_deref(),
        asserted.email.as_deref(),
        Some(asserted.subject.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(sanitize)
    .find(|s| !s.is_empty())
    .unwrap_or_else(|| FALLBACK.to_owned())
}

/// `base`, then `base-2` … `base-MAX_SUFFIX`, each a valid username.
pub(crate) fn candidates(base: &str) -> impl Iterator<Item = String> + '_ {
    std::iter::once(base.to_owned())
        .chain((2..=MAX_SUFFIX).map(move |n| {
            let suffix = format!("-{n}");
            let keep = USERNAME_MAX_LEN - suffix.len();
            let stem: String = base.chars().take(keep).collect();
            format!("{stem}{suffix}")
        }))
        .filter(|name| username_norm(name).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asserted(preferred: Option<&str>, email: Option<&str>, sub: &str) -> Asserted {
        Asserted {
            subject: sub.to_owned(),
            email: email.map(str::to_owned),
            email_verified: None,
            preferred_username: preferred.map(str::to_owned),
            name: None,
            groups: Vec::new(),
        }
    }

    #[test]
    fn base_prefers_preferred_username_then_email_then_sub() {
        assert_eq!(
            base(&asserted(Some("alice"), Some("a@x.io"), "s1")),
            "alice"
        );
        assert_eq!(
            base(&asserted(None, Some("Bob@Example.com"), "s1")),
            "Bob@Example.com"
        );
        assert_eq!(base(&asserted(Some("张三"), None, "AbC-123")), "AbC-123");
        assert_eq!(base(&asserted(Some("Zoë Smith"), None, "s")), "ZoSmith");
        assert_eq!(base(&asserted(Some("  "), Some("@@@é"), "!!")), "@@@");
        assert_eq!(base(&asserted(None, None, "ü")), FALLBACK);
    }

    #[test]
    fn every_candidate_is_a_valid_username_within_64_characters() {
        let long = "a".repeat(200);
        let base_name = base(&asserted(Some(&long), None, "s"));
        assert_eq!(base_name.len(), USERNAME_MAX_LEN);

        let all: Vec<String> = candidates(&base_name).collect();

        assert_eq!(all.len(), MAX_SUFFIX as usize);
        assert!(all.iter().all(|n| username_norm(n).is_some()), "{all:?}");
        assert_eq!(all[1], format!("{}-2", "a".repeat(USERNAME_MAX_LEN - 2)));
        assert!(all.last().unwrap().ends_with(&format!("-{MAX_SUFFIX}")));
    }

    #[test]
    fn short_bases_keep_their_name() {
        let all: Vec<String> = candidates("alice").take(3).collect();
        assert_eq!(all, ["alice", "alice-2", "alice-3"]);
    }
}
