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

/// Escape character of `LIKE` patterns built from user input.
const LIKE_ESCAPE: char = '\\';

/// A `LIKE` pattern matching any value that contains `needle` literally (`%`, `_` and the
/// escape character in it are escaped). Lowercase both sides for a case-insensitive match.
pub(crate) fn contains_pattern(needle: &str) -> sea_orm::sea_query::LikeExpr {
    let mut pattern = String::with_capacity(needle.len() + 2);
    pattern.push('%');
    for c in needle.chars() {
        if matches!(c, '%' | '_' | LIKE_ESCAPE) {
            pattern.push(LIKE_ESCAPE);
        }
        pattern.push(c);
    }
    pattern.push('%');
    sea_orm::sea_query::LikeExpr::new(pattern).escape(LIKE_ESCAPE)
}
