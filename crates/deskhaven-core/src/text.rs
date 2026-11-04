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

/// Characters dropped from self-reported device text: control characters, bidi embedding /
/// override / isolate controls (which can make a hostname read as something else in the
/// console, "Trojan Source") and zero-width characters (which make look-alike names).
pub(crate) fn is_dropped(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}' // ALM (Arabic letter mark, a bidi mark)
                | '\u{180E}' // Mongolian vowel separator (invisible)
                | '\u{200B}'..='\u{200F}' // zero-width space / (non-)joiner, LRM, RLM
                | '\u{202A}'..='\u{202E}' // LRE, RLE, PDF, LRO, RLO
                | '\u{2060}'..='\u{2064}' // word joiner, invisible operators
                | '\u{2066}'..='\u{2069}' // LRI, RLI, FSI, PDI
                | '\u{FEFF}' // zero-width no-break space (BOM)
        )
}

/// `raw` without [`is_dropped`] characters (newline and tab kept when `multiline`), cut to
/// `max` characters: self-reported text is stored cleaned rather than refused.
pub(crate) fn clean(raw: &str, max: usize, multiline: bool) -> String {
    let kept = |c: &char| !is_dropped(*c) || (multiline && matches!(c, '\n' | '\t'));
    raw.chars().filter(kept).take(max).collect()
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
