//! Validation and normalization of user input (04 §3).
//!
//! - Username: 1-64 characters of `[A-Za-z0-9._@-]` (the schema CHECKs the same set);
//!   `username_norm` is its ASCII lowercase.
//! - Email: kept as entered; `email_norm` is the **whole address** with its ASCII letters
//!   lowercased. RFC 5321 allows a case-sensitive local part, but no mainstream provider uses
//!   one, and folding it prevents two accounts (or an OIDC link) differing only in case.
//!   Non-ASCII characters are compared as entered (M4-05): Unicode `to_lowercase` maps some of
//!   them onto ASCII letters (the Kelvin sign U+212A becomes `k`), so an IdP that asserts
//!   `\u{212A}evin@corp.example` as verified could otherwise be linked to `kevin@corp.example`.
//!   Validation is deliberately shallow: at most 254 characters, no whitespace or control
//!   characters, exactly one `@` with a non-empty part on each side. Deliverability is
//!   proven by verification, not by syntax. No Unicode normalization (NFC / NFKC) or case
//!   folding is applied: differently composed or cased forms of the same internationalized
//!   address are distinct. Accepted trade-off (no extra dependency; such addresses are rare,
//!   and a false "distinct" only fails to link, never links the wrong account).

use super::UserError;
use crate::text::{self, NOTE_MAX_LEN};

pub(crate) const USERNAME_MAX_LEN: usize = 64;
const EMAIL_MAX_LEN: usize = 254;
const DISPLAY_NAME_MAX_LEN: usize = 128;

fn invalid(field: &'static str, reason: impl Into<String>) -> UserError {
    UserError::Invalid {
        field,
        reason: reason.into(),
    }
}

pub(crate) fn is_username_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-')
}

/// `Some(username_norm)` when `raw` is a well-formed username.
pub fn username_norm(raw: &str) -> Option<String> {
    let well_formed =
        !raw.is_empty() && raw.len() <= USERNAME_MAX_LEN && raw.chars().all(is_username_char);
    well_formed.then(|| raw.to_ascii_lowercase())
}

pub(super) fn username(raw: &str) -> Result<String, UserError> {
    username_norm(raw).ok_or_else(|| {
        invalid(
            "username",
            format!("must be 1-{USERNAME_MAX_LEN} characters of A-Z a-z 0-9 . _ @ -"),
        )
    })
}

/// An email address and its normalized form.
pub(super) struct Email {
    pub value: String,
    pub norm: String,
}

pub(super) fn email(raw: &str) -> Result<Email, UserError> {
    let malformed = || invalid("email", "must be a single address of the form local@domain");
    if raw.chars().count() > EMAIL_MAX_LEN {
        return Err(invalid(
            "email",
            format!("must be at most {EMAIL_MAX_LEN} characters"),
        ));
    }
    if raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(malformed());
    }
    match raw.split_once('@') {
        Some((local, domain))
            if !local.is_empty() && !domain.is_empty() && !domain.contains('@') =>
        {
            Ok(Email {
                value: raw.to_owned(),
                norm: raw.to_ascii_lowercase(),
            })
        }
        _ => Err(malformed()),
    }
}

fn text(field: &'static str, raw: &str, max: usize, multiline: bool) -> Result<String, UserError> {
    match text::problem(raw, max, multiline) {
        Some(reason) => Err(invalid(field, reason)),
        None => Ok(raw.to_owned()),
    }
}

/// Trimmed display name; blank falls back to `username`.
pub(super) fn display_name(raw: &str, username: &str) -> Result<String, UserError> {
    let trimmed = raw.trim();
    let name = if trimmed.is_empty() {
        username
    } else {
        trimmed
    };
    text("display_name", name, DISPLAY_NAME_MAX_LEN, false)
}

pub(super) fn note(raw: &str) -> Result<String, UserError> {
    text("note", raw, NOTE_MAX_LEN, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_norm_is_ascii_lowercase() {
        assert_eq!(
            username_norm("Alice.Smith@Corp").as_deref(),
            Some("alice.smith@corp")
        );
        assert_eq!(username_norm(""), None);
        assert_eq!(username_norm("ünïcode"), None);
        assert_eq!(username_norm(&"a".repeat(USERNAME_MAX_LEN + 1)), None);
    }

    #[test]
    fn email_norm_lowercases_ascii_letters_of_the_whole_address_only() {
        let norm = |raw: &str| email(raw).ok().map(|e| e.norm);

        assert_eq!(
            norm("Ünïcode.User@Example.COM").as_deref(),
            Some("Ünïcode.user@example.com")
        );
        // Unicode lowercasing maps the Kelvin sign (U+212A) to ASCII `k`; ASCII-only folding
        // keeps such an address distinct from the ASCII one it imitates.
        assert_eq!(
            norm("\u{212A}evin@corp.example").as_deref(),
            Some("\u{212A}evin@corp.example")
        );
        assert_ne!(
            norm("\u{212A}evin@corp.example"),
            norm("kevin@corp.example")
        );
    }

    #[test]
    fn display_name_falls_back_to_username() {
        assert_eq!(display_name("  ", "bob").ok().as_deref(), Some("bob"));
        assert_eq!(
            display_name(" Bob B. ", "bob").ok().as_deref(),
            Some("Bob B.")
        );
    }
}
