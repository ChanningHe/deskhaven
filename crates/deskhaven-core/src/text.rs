//! Shared checks for free-text input fields.

/// Longest accepted free-form note, in characters.
pub(crate) const NOTE_MAX_LEN: usize = 2000;

/// Unicode `Bidi_Control` characters: the marks, embeddings, overrides and isolates that
/// reorder how neighbouring text is displayed ("Trojan Source": U+202E turns `nimda` into
/// `admin` in the console). ZWJ / ZWNJ are not among them; emoji sequences and Persian or
/// Indic text need those.
pub(crate) fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' // ALM (Arabic letter mark)
            | '\u{200E}'..='\u{200F}' // LRM, RLM
            | '\u{202A}'..='\u{202E}' // LRE, RLE, PDF, LRO, RLO
            | '\u{2066}'..='\u{2069}' // LRI, RLI, FSI, PDI
    )
}

/// Why `raw` is not acceptable as a text field of at most `max` characters without control
/// or bidirectional formatting characters (newline and tab allowed when `multiline`), or
/// `None` when it is.
pub(crate) fn problem(raw: &str, max: usize, multiline: bool) -> Option<String> {
    if raw.chars().count() > max {
        return Some(format!("must be at most {max} characters"));
    }
    let bad = |c: char| {
        (c.is_control() && !(multiline && matches!(c, '\n' | '\t'))) || is_bidi_control(c)
    };
    raw.chars()
        .any(bad)
        .then(|| "must not contain control or bidirectional formatting characters".to_owned())
}

/// Zero-width characters other than the joiners: invisible, so they make look-alike names.
fn is_zero_width_filler(c: char) -> bool {
    matches!(
        c,
        '\u{180E}' // Mongolian vowel separator (invisible)
            | '\u{200B}' // zero-width space
            | '\u{2060}'
            ..='\u{2064}' // word joiner, invisible operators
            | '\u{FEFF}' // zero-width no-break space (BOM)
    )
}

/// Characters dropped from self-reported device text: control characters, [`is_bidi_control`]
/// characters (which can make a hostname read as something else in the console) and
/// zero-width characters, joiners included (which make look-alike names).
pub(crate) fn is_dropped(c: char) -> bool {
    c.is_control()
        || is_bidi_control(c)
        || is_zero_width_filler(c)
        || matches!(c, '\u{200C}' | '\u{200D}') // zero-width non-joiner / joiner
}

/// `raw` without control and [`is_bidi_control`] characters (newline and tab kept when
/// `multiline`), cut to `max` characters: data pushed by the RustDesk client is stored
/// cleaned rather than refused, so one odd peer name cannot fail a whole sync. Unlike
/// [`clean`] and [`clean_import`] it keeps joiners and zero-width spaces.
pub(crate) fn strip_bidi_and_control(raw: &str, max: usize, multiline: bool) -> String {
    let kept = |c: &char| {
        let control = c.is_control() && !(multiline && matches!(c, '\n' | '\t'));
        !control && !is_bidi_control(*c)
    };
    raw.chars().filter(kept).take(max).collect()
}

/// `raw` without control, [`is_bidi_control`] and zero-width characters, cut to `max`
/// characters: the one cleaning of names imported from an identity provider (OIDC claims,
/// LDAP attributes), cleaned rather than refused so a login never fails over a name. The
/// joiners stay, so emoji sequences and Persian / Indic names stay whole.
pub(crate) fn clean_import(raw: &str, max: usize) -> String {
    let kept = |c: &char| !c.is_control() && !is_bidi_control(*c) && !is_zero_width_filler(*c);
    raw.chars().filter(kept).take(max).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    const BIDI_CONTROLS: [char; 12] = [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
        '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];

    #[test]
    fn problem_refuses_every_bidi_control() {
        for c in BIDI_CONTROLS {
            let raw = format!("a{c}b");

            assert!(is_bidi_control(c), "U+{:04X}", c as u32);
            assert!(problem(&raw, 64, false).is_some(), "U+{:04X}", c as u32);
            assert!(problem(&raw, 64, true).is_some(), "U+{:04X}", c as u32);
        }
    }

    #[test]
    fn problem_refuses_a_right_to_left_override_spoof() {
        assert!(problem("\u{202E}nimda", 64, false).is_some());
    }

    #[test]
    fn problem_accepts_joiners_emoji_sequences_and_scripts_that_need_them() {
        let accepted = [
            "a\u{200D}b",
            "a\u{200C}b",
            "\u{1F469}\u{200D}\u{1F4BB}",
            "\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}",
            "\u{5F20}\u{4E09}",
            "\u{05E9}\u{05DC}\u{05D5}\u{05DD}",
            "plain",
        ];
        for raw in accepted {
            assert_eq!(problem(raw, 64, false), None, "{raw:?}");
        }
    }

    #[test]
    fn problem_still_refuses_control_characters_and_overlong_text() {
        assert!(problem("a\u{0}b", 64, true).is_some());
        assert!(problem("a\nb", 64, false).is_some());
        assert_eq!(problem("a\nb\tc", 64, true), None);
        assert!(problem("abcd", 3, false).is_some());
    }

    #[test]
    fn dropped_text_covers_the_bidi_controls_and_zero_width_characters() {
        for c in BIDI_CONTROLS {
            assert!(is_dropped(c), "U+{:04X}", c as u32);
        }
        for c in ['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}', '\u{0}'] {
            assert!(is_dropped(c), "U+{:04X}", c as u32);
        }
        assert!(!is_dropped('a'));
    }

    #[test]
    fn imported_text_loses_bidi_control_and_zero_width_characters_but_keeps_joiners() {
        let raw = "\u{202E}Ann\u{0}\u{200F}a\u{200B}\u{FEFF}\u{2060}\u{180E}\u{200D}b\u{200C}c";

        assert_eq!(clean_import(raw, 64), "Anna\u{200D}b\u{200C}c");
        assert_eq!(clean_import("abcdef", 3), "abc");
        assert_eq!(problem(&clean_import(raw, 64), 64, false), None);
    }

    #[test]
    fn client_text_is_stripped_of_bidi_and_control_but_keeps_joiners_and_zero_width_spaces() {
        let raw = "\u{202E}Ann\u{0}\u{200F}a\u{200D}b\u{200C}c";

        assert_eq!(
            strip_bidi_and_control(raw, 64, false),
            "Anna\u{200D}b\u{200C}c"
        );
        assert_eq!(
            strip_bidi_and_control("a\u{200B}b", 64, false),
            "a\u{200B}b"
        );
        assert_eq!(strip_bidi_and_control("abcdef", 3, false), "abc");
        assert_eq!(
            problem(&strip_bidi_and_control(raw, 64, false), 64, false),
            None
        );
        assert_eq!(
            strip_bidi_and_control("a\n\u{202E}b\tc\u{0}", 64, true),
            "a\nb\tc"
        );
        assert_eq!(strip_bidi_and_control("a\nb\tc", 64, false), "abc");
    }
}
