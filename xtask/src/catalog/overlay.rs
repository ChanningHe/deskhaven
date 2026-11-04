//! The human-written half of the catalog (`overlay.toml`): group, control type, choices,
//! defaults the source does not imply, secrecy and the en / zh texts. Merged onto the keys
//! read from the client source; a key the overlay does not describe still appears (as a
//! text option in `other`), an overlay entry whose key left the source is an error.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::source::SourceKey;

pub const GROUPS: &[&str] = &[
    "security",
    "permissions",
    "display",
    "network",
    "recording",
    "presets",
    "other",
];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OverlayError {
    #[error("overlay.toml is not valid: {0}")]
    Syntax(String),
    #[error("overlay entry `{0}` is not in the client's key list (remove it)")]
    Stale(String),
    #[error("option `{key}`: {reason}")]
    Invalid { key: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Text {
    pub en: String,
    pub zh: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub value: String,
    pub en: String,
    pub zh: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revisions {
    pub rustdesk: String,
    pub hbb_common: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverlayFile {
    source: Revisions,
    options: BTreeMap<String, OverlayOption>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverlayOption {
    group: String,
    kind: String,
    #[serde(default)]
    default: Option<String>,
    /// Required (true or false) when the name suggests a credential.
    #[serde(default)]
    secret: Option<bool>,
    /// Left out of the typed form; set only through the advanced editor.
    #[serde(default)]
    advanced_only: bool,
    #[serde(default)]
    choices: Vec<Choice>,
    #[serde(default)]
    min: Option<i64>,
    #[serde(default)]
    max: Option<i64>,
    label: Text,
    description: Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Choice(Vec<Choice>),
    Number { min: Option<i64>, max: Option<i64> },
    Text,
}

/// One option of the generated catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    pub kind: Kind,
    pub group: String,
    pub default: Option<String>,
    pub secret: bool,
    pub advanced_only: bool,
    pub label: Text,
    pub description: Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub source: Revisions,
    pub entries: Vec<Entry>,
}

/// `keys` (source order) with the overlay `toml` merged on.
pub fn merge(keys: &[SourceKey], toml: &str) -> Result<Catalog, OverlayError> {
    let file: OverlayFile =
        toml::from_str(toml).map_err(|e| OverlayError::Syntax(e.message().to_owned()))?;
    let mut options = file.options;
    let entries = keys
        .iter()
        .map(|k| match options.remove(&k.key) {
            Some(o) => entry(k, o),
            None => undescribed(k),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(stale) = options.into_keys().next() {
        return Err(OverlayError::Stale(stale));
    }
    Ok(Catalog {
        source: file.source,
        entries,
    })
}

/// Name segments (split at `-` / `_`) that suggest a credential: one containing `pass`,
/// `secret` or `token`, or the segment `key`.
fn looks_secret(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.split(['-', '_']).any(|segment| {
        segment == "key"
            || ["pass", "secret", "token"]
                .iter()
                .any(|f| segment.contains(f))
    })
}

const SECRET_UNDECIDED: &str = "the name suggests a credential: set `secret` to true or false";

/// A key the overlay does not describe yet: a text option in `other`, unless its name
/// suggests a credential (then the overlay must decide).
fn undescribed(k: &SourceKey) -> Result<Entry, OverlayError> {
    if looks_secret(&k.key) {
        return Err(OverlayError::Invalid {
            key: k.key.clone(),
            reason: SECRET_UNDECIDED.to_owned(),
        });
    }
    Ok(Entry {
        key: k.key.clone(),
        kind: match k.default {
            Some(_) => Kind::Bool,
            None => Kind::Text,
        },
        group: "other".to_owned(),
        default: k.default.map(str::to_owned),
        secret: false,
        advanced_only: false,
        label: Text {
            en: k.key.clone(),
            zh: k.key.clone(),
        },
        description: Text {
            en: String::new(),
            zh: String::new(),
        },
    })
}

fn entry(k: &SourceKey, o: OverlayOption) -> Result<Entry, OverlayError> {
    let invalid = |reason: &str| OverlayError::Invalid {
        key: k.key.clone(),
        reason: reason.to_owned(),
    };
    if !GROUPS.contains(&o.group.as_str()) {
        return Err(invalid("unknown group"));
    }
    let default = match (k.default, o.default) {
        (Some(_), Some(_)) => return Err(invalid("the client source sets the default; drop it")),
        (source, overlay) => overlay.or_else(|| source.map(str::to_owned)),
    };
    let kind = kind(&o.kind, o.choices, o.min, o.max).map_err(|r| invalid(&r))?;
    if let Some(value) = &default {
        check_default(&kind, value).map_err(|r| invalid(&r))?;
    }
    let secret = match (o.secret, looks_secret(&k.key)) {
        (None, true) => return Err(invalid(SECRET_UNDECIDED)),
        (secret, _) => secret.unwrap_or(false),
    };
    if secret && kind != Kind::Text {
        return Err(invalid("only text options can be secret"));
    }
    Ok(Entry {
        key: k.key.clone(),
        kind,
        group: o.group,
        default,
        secret,
        advanced_only: o.advanced_only,
        label: o.label,
        description: o.description,
    })
}

fn kind(
    name: &str,
    choices: Vec<Choice>,
    min: Option<i64>,
    max: Option<i64>,
) -> Result<Kind, String> {
    let has_range = min.is_some() || max.is_some();
    match name {
        "choice" if choices.is_empty() => Err("a choice option needs choices".to_owned()),
        "choice" if choices.iter().any(|c| c.value.is_empty()) => {
            Err("an empty choice would delete the option".to_owned())
        }
        "choice" if !has_range => Ok(Kind::Choice(choices)),
        "number" if choices.is_empty() => match (min, max) {
            (Some(lo), Some(hi)) if lo > hi => Err("min is above max".to_owned()),
            _ => Ok(Kind::Number { min, max }),
        },
        "bool" | "text" if choices.is_empty() && !has_range => Ok(match name {
            "bool" => Kind::Bool,
            _ => Kind::Text,
        }),
        "bool" | "text" | "choice" | "number" => Err("fields that do not fit the kind".to_owned()),
        _ => Err(format!("unknown kind `{name}`")),
    }
}

fn check_default(kind: &Kind, value: &str) -> Result<(), String> {
    let fits = match kind {
        Kind::Bool => value == "Y" || value == "N",
        Kind::Choice(choices) => choices.iter().any(|c| c.value == value),
        Kind::Number { min, max } => value
            .parse::<i64>()
            .is_ok_and(|n| min.is_none_or(|lo| n >= lo) && max.is_none_or(|hi| n <= hi)),
        Kind::Text => true,
    };
    match fits {
        true => Ok(()),
        false => Err(format!("default `{value}` does not fit the kind")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "[source]\nrustdesk = \"r1\"\nhbb_common = \"h1\"\n";

    fn keys() -> Vec<SourceKey> {
        let key = |key: &str, default| SourceKey {
            key: key.to_owned(),
            default,
        };
        vec![
            key("enable-keyboard", Some("Y")),
            key("approve-mode", None),
            key("whitelist", None),
        ]
    }

    fn option(key: &str, body: &str) -> String {
        format!(
            "[options.{key}]\n{body}\nlabel = {{ en = \"L\", zh = \"标\" }}\ndescription = {{ en = \"D\", zh = \"述\" }}\n"
        )
    }

    fn merged(options: &[String]) -> Result<Catalog, OverlayError> {
        merge(&keys(), &format!("{HEAD}{}", options.concat()))
    }

    fn invalid(result: Result<Catalog, OverlayError>) -> String {
        match result {
            Err(OverlayError::Invalid { reason, .. }) => reason,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn credential_like_names_need_an_explicit_secret_decision() {
        assert!(looks_secret("proxy-password") && looks_secret("key"));
        assert!(looks_secret("api_token") && looks_secret("client-secret"));
        assert!(!looks_secret("enable-keyboard") && !looks_secret("whitelist"));
        let with = |list: &[&str]| -> Vec<SourceKey> {
            list.iter()
                .map(|k| SourceKey {
                    key: (*k).to_owned(),
                    default: None,
                })
                .collect()
        };

        let undescribed = merge(&with(&["proxy-password"]), &format!("{HEAD}[options]\n"));
        assert_eq!(invalid(undescribed), SECRET_UNDECIDED);
        let text = "group = \"network\"\nkind = \"text\"";
        let silent = format!("{HEAD}{}", option("key", text));
        assert_eq!(invalid(merge(&with(&["key"]), &silent)), SECRET_UNDECIDED);
        let decided = format!(
            "{HEAD}{}",
            option(
                "key",
                &format!("{text}\nsecret = false\nadvanced_only = true")
            )
        );
        let entry = &merge(&with(&["key"]), &decided).unwrap().entries[0];
        assert!(!entry.secret && entry.advanced_only);
    }

    #[test]
    fn overlay_refines_source_keys_and_undescribed_keys_still_appear() {
        let catalog = merged(&[
            option(
                "approve-mode",
                "group = \"security\"\nkind = \"choice\"\ndefault = \"both\"\nchoices = [{ value = \"click\", en = \"Click\", zh = \"点击\" }, { value = \"both\", en = \"Both\", zh = \"都\" }]",
            ),
            option(
                "enable-keyboard",
                "group = \"permissions\"\nkind = \"bool\"",
            ),
        ])
        .unwrap();

        assert_eq!(catalog.source.rustdesk, "r1");
        let keys: Vec<_> = catalog.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["enable-keyboard", "approve-mode", "whitelist"]);
        let keyboard = &catalog.entries[0];
        assert_eq!(keyboard.default.as_deref(), Some("Y"), "from the source");
        assert_eq!(keyboard.group, "permissions");
        assert_eq!(keyboard.label.zh, "标");
        let approve = &catalog.entries[1];
        assert_eq!(approve.default.as_deref(), Some("both"), "from the overlay");
        assert!(matches!(&approve.kind, Kind::Choice(c) if c.len() == 2));
        let whitelist = &catalog.entries[2];
        assert_eq!(
            (whitelist.kind.clone(), whitelist.group.as_str()),
            (Kind::Text, "other")
        );
        assert_eq!(whitelist.label.en, "whitelist");
    }

    #[test]
    fn stale_entries_and_syntax_errors_are_reported() {
        let stale = merged(&[option("gone-key", "group = \"other\"\nkind = \"text\"")]);
        assert_eq!(stale, Err(OverlayError::Stale("gone-key".to_owned())));
        assert!(matches!(
            merge(&keys(), "[source]\nrustdesk = 1"),
            Err(OverlayError::Syntax(_))
        ));
        let unknown_field = option(
            "whitelist",
            "group = \"other\"\nkind = \"text\"\ncolour = 1",
        );
        assert!(matches!(
            merged(&[unknown_field]),
            Err(OverlayError::Syntax(_))
        ));
    }

    #[test]
    fn inconsistent_entries_are_rejected() {
        let cases = [
            (
                "enable-keyboard",
                "group = \"permissions\"\nkind = \"bool\"\ndefault = \"N\"",
                "the client source sets the default; drop it",
            ),
            (
                "whitelist",
                "group = \"nowhere\"\nkind = \"text\"",
                "unknown group",
            ),
            (
                "whitelist",
                "group = \"other\"\nkind = \"list\"",
                "unknown kind `list`",
            ),
            (
                "whitelist",
                "group = \"other\"\nkind = \"choice\"",
                "a choice option needs choices",
            ),
            (
                "whitelist",
                "group = \"other\"\nkind = \"choice\"\nchoices = [{ value = \"\", en = \"x\", zh = \"x\" }]",
                "an empty choice would delete the option",
            ),
            (
                "whitelist",
                "group = \"other\"\nkind = \"number\"\nmin = 5\nmax = 1",
                "min is above max",
            ),
            (
                "whitelist",
                "group = \"other\"\nkind = \"number\"\nmin = 1\nmax = 9\ndefault = \"10\"",
                "default `10` does not fit the kind",
            ),
            (
                "whitelist",
                "group = \"other\"\nkind = \"bool\"\ndefault = \"yes\"",
                "default `yes` does not fit the kind",
            ),
            (
                "whitelist",
                "group = \"other\"\nkind = \"text\"\nmin = 1",
                "fields that do not fit the kind",
            ),
            (
                "approve-mode",
                "group = \"other\"\nkind = \"bool\"\nsecret = true",
                "only text options can be secret",
            ),
        ];
        for (key, body, reason) in cases {
            assert_eq!(invalid(merged(&[option(key, body)])), reason, "{body}");
        }
    }
}
