//! Shared checks for free-text input fields.

/// Longest accepted free-form note, in characters.
pub(crate) const NOTE_MAX_LEN: usize = 2000;

/// Why `raw` is not acceptable as a text field of at most `max` characters without control
/// characters (newline and tab allowed when `multiline`), or `None` when it is.
pub(crate) fn problem(raw: &str, max: usize, multiline: bool) -> Option<String> {
    if raw.chars().count() > max {
        return Some(format!("must be at most {max} characters"));
    }
    let bad = |c: char| c.is_control() && !(multiline && matches!(c, '\n' | '\t'));
    raw.chars()
        .any(bad)
        .then(|| "must not contain control characters".to_owned())
}
