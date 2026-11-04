//! Search filters and DNs (06 §3.5): every value spliced into a filter is RFC 4515-escaped
//! (`*`, `(`, `)`, `\` and NUL become `\2a`, `\28`, `\29`, `\5c`, `\00`), placeholders are
//! substituted in one pass (a value is never scanned for further placeholders), and DNs
//! compare by RDN, ASCII case-insensitively.

/// Placeholder of a group filter for the user's DN.
pub const USER_DN_PLACEHOLDER: &str = "{user_dn}";
/// Placeholder of a group filter for the user's username attribute value.
pub const USERNAME_PLACEHOLDER: &str = "{username}";
/// Group filter when none is configured: the usual membership attributes.
pub const DEFAULT_GROUP_FILTER: &str =
    "(|(member={user_dn})(uniqueMember={user_dn})(memberUid={username}))";

/// `raw` as an RFC 4515 assertion value.
pub fn escape_filter_value(raw: &str) -> String {
    ldap3::ldap_escape(raw).into_owned()
}

/// Whether `filter` is one well-formed RFC 4515 filter.
pub(crate) fn is_valid_filter(filter: &str) -> bool {
    ldap3::parse_filter(filter).is_ok()
}

/// `(&<user_filter>(<attr>=<escaped username>))`.
pub(crate) fn user_search(user_filter: &str, attr: &str, username: &str) -> String {
    format!("(&{user_filter}({attr}={}))", escape_filter_value(username))
}

/// `template` with [`USER_DN_PLACEHOLDER`] and [`USERNAME_PLACEHOLDER`] replaced by the
/// escaped values.
pub(crate) fn group_search(template: &str, user_dn: &str, username: &str) -> String {
    let user_dn = escape_filter_value(user_dn);
    let username = escape_filter_value(username);
    let mut out = String::with_capacity(template.len() + user_dn.len());
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix(USER_DN_PLACEHOLDER) {
            out.push_str(&user_dn);
            rest = after;
        } else if let Some(after) = tail.strip_prefix(USERNAME_PLACEHOLDER) {
            out.push_str(&username);
            rest = after;
        } else {
            out.push('{');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Splits `s` at every `sep` that is not backslash-escaped.
fn split_unescaped(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            c if c == sep => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// A comparison key: RDNs trimmed, ASCII-lowercased, joined by `,`.
pub(crate) fn dn_key(dn: &str) -> String {
    split_unescaped(dn, ',')
        .into_iter()
        .map(|rdn| rdn.trim().to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join(",")
}

/// Whether `dn` lies strictly below `base`.
pub(crate) fn is_under(dn: &str, base: &str) -> bool {
    let (dn, base) = (dn_key(dn), dn_key(base));
    dn.len() > base.len() && dn.ends_with(&base) && dn[..dn.len() - base.len()].ends_with(',')
}

/// The value of the first attribute of the first RDN, unescaped (`cn=ops,ou=groups,...`
/// -> `ops`); `None` when it is empty or not UTF-8.
pub(crate) fn first_rdn_value(dn: &str) -> Option<String> {
    let rdn = split_unescaped(dn, ',').into_iter().next()?;
    let ava = split_unescaped(rdn, '+').into_iter().next()?;
    let (_, value) = ava.split_once('=')?;
    let value = unescape_dn_value(value.trim())?;
    (!value.is_empty()).then_some(value)
}

/// RFC 4514 unescaping: `\XX` is a byte, `\c` is `c`.
fn unescape_dn_value(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let hex = bytes.get(i + 1..i + 3).and_then(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        });
        match (hex, bytes.get(i + 1)) {
            (Some(byte), _) => {
                out.push(byte);
                i += 3;
            }
            (None, Some(&next)) => {
                out.push(next);
                i += 2;
            }
            (None, None) => return None,
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use ldap3::asn1::{ASNTag, PL, StructureTag};
    use proptest::prelude::*;

    use super::*;

    /// RFC 4511 Filter tags (context-specific).
    const AND: u64 = 0;
    const EQUALITY: u64 = 3;

    fn children(tag: &StructureTag) -> &[StructureTag] {
        match &tag.payload {
            PL::C(children) => children,
            PL::P(_) => panic!("primitive tag {tag:?}"),
        }
    }

    fn bytes(tag: &StructureTag) -> &[u8] {
        match &tag.payload {
            PL::P(bytes) => bytes,
            PL::C(_) => panic!("constructed tag {tag:?}"),
        }
    }

    /// The equality assertion `(attr=value)` of the composed user search, parsed back.
    fn assertion_of(filter: &str) -> (Vec<u8>, Vec<u8>) {
        let parsed = ldap3::parse_filter(filter).unwrap().into_structure();
        assert_eq!(parsed.id, AND, "{filter}");
        let parts = children(&parsed);
        assert_eq!(parts.len(), 2, "{filter}");
        assert_eq!(parts[1].id, EQUALITY, "{filter}");
        let ava = children(&parts[1]);
        (bytes(&ava[0]).to_vec(), bytes(&ava[1]).to_vec())
    }

    fn specials() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just('*'),
                Just('('),
                Just(')'),
                Just('\\'),
                Just('\0'),
                Just('='),
                Just('{'),
                Just('}'),
                any::<char>(),
            ],
            0..24,
        )
        .prop_map(|chars| chars.into_iter().collect())
    }

    proptest! {
        #[test]
        fn escaped_values_carry_no_filter_metacharacters(raw in specials()) {
            let escaped = escape_filter_value(&raw);

            prop_assert!(!escaped.contains(['*', '(', ')', '\0']), "{escaped:?}");
            // Every backslash starts a two-digit hex escape.
            let bytes = escaped.as_bytes();
            for (i, b) in bytes.iter().enumerate() {
                if *b == b'\\' {
                    let pair = bytes.get(i + 1..i + 3).unwrap_or_default();
                    prop_assert!(pair.len() == 2 && pair.iter().all(u8::is_ascii_hexdigit));
                }
            }
            prop_assert_eq!(ldap3::ldap_unescape(escaped.as_str()).unwrap(), raw.as_str());
        }

        #[test]
        fn any_username_stays_one_equality_assertion(raw in specials()) {
            let filter = user_search("(objectClass=person)", "uid", &raw);

            let (attr, value) = assertion_of(&filter);

            prop_assert_eq!(attr, b"uid".to_vec());
            prop_assert_eq!(value, raw.as_bytes().to_vec());
        }

        #[test]
        fn group_templates_substitute_escaped_values_once(dn in specials(), name in specials()) {
            let filter = group_search(DEFAULT_GROUP_FILTER, &dn, &name);

            prop_assert!(is_valid_filter(&filter), "{filter:?}");
            let parsed = ldap3::parse_filter(&filter).unwrap().into_structure();
            let alternatives = children(&parsed);
            prop_assert_eq!(alternatives.len(), 3);
            let values: Vec<Vec<u8>> = alternatives
                .iter()
                .map(|alt| bytes(&children(alt)[1]).to_vec())
                .collect();
            prop_assert_eq!(&values[0], &dn.as_bytes().to_vec());
            prop_assert_eq!(&values[1], &dn.as_bytes().to_vec());
            prop_assert_eq!(&values[2], &name.as_bytes().to_vec());
        }
    }

    #[test]
    fn the_rfc_4515_specials_are_hex_escaped() {
        assert_eq!(
            escape_filter_value("a*b(c)d\\e\0f"),
            "a\\2ab\\28c\\29d\\5ce\\00f"
        );
        assert_eq!(
            user_search("(objectClass=person)", "uid", "*)(uid=*"),
            "(&(objectClass=person)(uid=\\2a\\29\\28uid=\\2a))"
        );
    }

    #[test]
    fn placeholders_inside_values_are_not_expanded() {
        let filter = group_search("(&(member={user_dn})(x={username}))", "cn={username}", "n");
        assert_eq!(filter, "(&(member=cn={username})(x=n))");
        assert_eq!(group_search("(cn={other})", "d", "n"), "(cn={other})");
    }

    #[test]
    fn dns_compare_by_rdn_case_insensitively() {
        assert_eq!(
            dn_key("CN=Ops, OU=Groups,dc=Example"),
            "cn=ops,ou=groups,dc=example"
        );
        assert!(is_under("cn=ops,ou=groups,dc=x", "OU=Groups, DC=X"));
        assert!(!is_under("ou=groups,dc=x", "ou=groups,dc=x"));
        assert!(!is_under("cn=a,xou=groups,dc=x", "ou=groups,dc=x"));
        assert!(!is_under("cn=a\\,ou=groups,dc=x", "ou=groups,dc=x,dc=y"));
    }

    #[test]
    fn group_names_come_from_the_first_rdn() {
        assert_eq!(
            first_rdn_value("cn=ops,ou=groups,dc=x").as_deref(),
            Some("ops")
        );
        assert_eq!(
            first_rdn_value("CN = Ops Team ,dc=x").as_deref(),
            Some("Ops Team")
        );
        assert_eq!(first_rdn_value("cn=a\\,b,dc=x").as_deref(), Some("a,b"));
        assert_eq!(
            first_rdn_value("cn=caf\\C3\\A9+uid=1,dc=x").as_deref(),
            Some("café")
        );
        assert_eq!(first_rdn_value("cn=,dc=x"), None);
        assert_eq!(first_rdn_value("noequals"), None);
        assert_eq!(first_rdn_value("cn=\\ff"), None);
    }
}
