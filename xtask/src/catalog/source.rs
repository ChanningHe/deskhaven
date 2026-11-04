//! Reads the client's option keys and their boolean defaults from its Rust source (read as
//! text, never built):
//!
//! - `rustdesk/libs/base/src/config/keys.rs`: the `OPTION_*` constants and `KEYS_SETTINGS`,
//!   the options the client keeps in `Config::options`, which is where a strategy's
//!   `config_options` land (`src/hbbs_http/sync.rs`, `handle_config_options`).
//! - `hbb_common/src/config.rs`: the `OPTION_*` constants of its `keys` module (re-exported
//!   by the client) and `option2bool`, whose branches give the default of `enable-*` (on)
//!   and `allow-*` style (off) options.

use std::collections::BTreeMap;

/// Where the files live under `--from`.
pub const CONFIG_RS: &str = "hbb_common/src/config.rs";
pub const KEYS_RS: &str = "rustdesk/libs/base/src/config/keys.rs";
/// The key list of the options a strategy can set.
pub const SETTINGS_LIST: &str = "KEYS_SETTINGS";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SourceError {
    #[error("`{0}` not found")]
    Missing(&'static str),
    #[error("`{0}` names no string constant")]
    Unresolved(String),
    #[error("unexpected shape in option2bool: `{0}`")]
    Shape(String),
}

/// One option of the key list with the default its name implies (`option2bool`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceKey {
    pub key: String,
    /// `"Y"` or `"N"` when an `option2bool` rule covers the key.
    pub default: Option<&'static str>,
}

/// A rule of `option2bool`: options it matches are on unless `"N"` (`Y`) or off unless
/// `"Y"` (`N`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    prefixes: Vec<String>,
    keys: Vec<String>,
    default: &'static str,
}

impl Rule {
    fn matches(&self, key: &str) -> bool {
        self.prefixes.iter().any(|p| key.starts_with(p.as_str()))
            || self.keys.iter().any(|k| k == key)
    }
}

/// The options of [`SETTINGS_LIST`] in source order, with their derived defaults.
pub fn settings_keys(config_rs: &str, keys_rs: &str) -> Result<Vec<SourceKey>, SourceError> {
    let shared = string_consts(
        block(config_rs, "pub mod keys {").ok_or(SourceError::Missing("pub mod keys"))?,
    );
    let mut consts = string_consts(keys_rs);
    consts.extend(shared);
    let rules = bool_rules(config_rs, &consts)?;
    let names = key_list(keys_rs, SETTINGS_LIST)?;
    names
        .iter()
        .map(|name| {
            let key = resolve(name, &consts)?;
            let default = rules.iter().find(|r| r.matches(&key)).map(|r| r.default);
            Ok(SourceKey { key, default })
        })
        .collect()
}

/// `pub const NAME: &str = "value";` items of `src` (one statement may span lines).
fn string_consts(src: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for item in src.split("pub const ").skip(1) {
        let Some((statement, _)) = item.split_once(';') else {
            continue;
        };
        let Some((name, rest)) = statement.split_once(':') else {
            continue;
        };
        let Some((ty, value)) = rest.split_once('=') else {
            continue;
        };
        if ty.trim() != "&str" {
            continue;
        }
        if let Some(value) = literal(value.trim()) {
            found.insert(name.trim().to_owned(), value);
        }
    }
    found
}

/// The content of a plain string literal (no escapes).
fn literal(token: &str) -> Option<String> {
    let inner = token.strip_prefix('"')?.strip_suffix('"')?;
    (!inner.contains(['"', '\\'])).then(|| inner.to_owned())
}

/// The text inside the braces that follow `opener` (which ends with `{`).
fn block<'a>(src: &'a str, opener: &str) -> Option<&'a str> {
    let start = src.find(opener)? + opener.len();
    let mut depth = 1usize;
    for (offset, c) in src[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[start..start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The identifiers (or literals) of `pub const <name>: &[&str] = &[...];`, comments dropped.
fn key_list(src: &str, name: &'static str) -> Result<Vec<String>, SourceError> {
    let opener = format!("pub const {name}: &[&str] = &[");
    let start = src.find(&opener).ok_or(SourceError::Missing(name))? + opener.len();
    let body = src[start..]
        .split_once("];")
        .ok_or(SourceError::Missing(name))?
        .0;
    let code: String = body
        .lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(code
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect())
}

/// A key list item: a constant name or a string literal.
fn resolve(item: &str, consts: &BTreeMap<String, String>) -> Result<String, SourceError> {
    let name = item.strip_prefix("keys::").unwrap_or(item);
    literal(item)
        .or_else(|| consts.get(name).cloned())
        .ok_or_else(|| SourceError::Unresolved(item.to_owned()))
}

/// The `if` / `else if` rules of `option2bool`, in order. The final `else` gives no rule:
/// it only holds for options the client reads through `option2bool`, which the name does
/// not tell.
fn bool_rules(
    config_rs: &str,
    consts: &BTreeMap<String, String>,
) -> Result<Vec<Rule>, SourceError> {
    let body = block(
        config_rs,
        "pub fn option2bool(option: &str, value: &str) -> bool {",
    )
    .ok_or(SourceError::Missing("fn option2bool"))?;
    let mut rules = Vec::new();
    for branch in body.split("} else") {
        let Some(condition) = branch.trim().strip_prefix("if ") else {
            continue;
        };
        let (condition, result) = condition
            .split_once('{')
            .ok_or_else(|| SourceError::Shape(branch.trim().to_owned()))?;
        let default = match result.trim() {
            r#"value != "N""# => "Y",
            r#"value == "Y""# => "N",
            other => return Err(SourceError::Shape(other.to_owned())),
        };
        rules.push(rule(condition, default, consts)?);
    }
    match rules.is_empty() {
        true => Err(SourceError::Shape(body.trim().to_owned())),
        false => Ok(rules),
    }
}

fn rule(
    condition: &str,
    default: &'static str,
    consts: &BTreeMap<String, String>,
) -> Result<Rule, SourceError> {
    let mut rule = Rule {
        prefixes: Vec::new(),
        keys: Vec::new(),
        default,
    };
    for alternative in condition.split("||").map(str::trim) {
        if let Some(arg) = alternative
            .strip_prefix("option.starts_with(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            let prefix = literal(arg.trim());
            rule.prefixes
                .push(prefix.ok_or_else(|| SourceError::Shape(alternative.to_owned()))?);
        } else if let Some(operand) = alternative.strip_prefix("option == ") {
            rule.keys.push(resolve(operand.trim(), consts)?);
        } else {
            return Err(SourceError::Shape(alternative.to_owned()));
        }
    }
    Ok(rule)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = include_str!("../../fixtures/strategy-catalog/hbb_common/src/config.rs");
    const KEYS: &str =
        include_str!("../../fixtures/strategy-catalog/rustdesk/libs/base/src/config/keys.rs");

    fn key(key: &str, default: Option<&'static str>) -> SourceKey {
        SourceKey {
            key: key.to_owned(),
            default,
        }
    }

    #[test]
    fn reads_the_settings_list_in_order_resolving_both_crates_constants() {
        let keys = settings_keys(CONFIG, KEYS).unwrap();

        assert_eq!(
            keys,
            [
                key("enable-keyboard", Some("Y")),
                key("approve-mode", None),
                key("direct-server", Some("N")),
                key("whitelist", None),
                key("proxy-password", None),
                key("allow-websocket", Some("N")),
                key("disable-udp", None),
            ]
        );
    }

    #[test]
    fn string_constants_span_lines_and_skip_other_types() {
        let consts = string_consts(KEYS);

        assert_eq!(
            consts["OPTION_HIDE_ELEVATE_BUTTON_IN_ACCEPT_WINDOW"],
            "hide-elevate-button-in-accept-window"
        );
        assert!(!consts.contains_key("KEYS_SETTINGS"));
        let shared = string_consts(block(CONFIG, "pub mod keys {").unwrap());
        assert_eq!(shared["OPTION_DISABLE_CHANGE_ID"], "disable-change-id");
        assert!(!shared.contains_key("NOT_A_KEY"), "outside the keys module");
    }

    #[test]
    fn option2bool_rules_come_from_its_branches() {
        let consts = string_consts(block(CONFIG, "pub mod keys {").unwrap());
        let rules = bool_rules(CONFIG, &consts).unwrap();

        assert_eq!(rules.len(), 2, "the final else derives nothing");
        assert_eq!(rules[0].prefixes, ["enable-"]);
        assert_eq!(rules[0].default, "Y");
        assert_eq!(rules[1].prefixes, ["allow-"]);
        assert_eq!(rules[1].keys, ["stop-service", "direct-server"]);
        assert_eq!(rules[1].default, "N");
    }

    #[test]
    fn unknown_shapes_and_names_fail_loudly() {
        let changed = CONFIG.replace(r#"value == "Y""#, r#"value == "1""#);
        assert!(matches!(
            settings_keys(&changed, KEYS),
            Err(SourceError::Shape(_))
        ));
        let regex = CONFIG.replace("option == \"stop-service\"", "is_stop(option)");
        assert!(matches!(
            settings_keys(&regex, KEYS),
            Err(SourceError::Shape(_))
        ));
        let typo = KEYS.replace(
            "    OPTION_WHITELIST,\n    OPTION_PROXY",
            "    OPTION_WHITELST,\n    OPTION_PROXY",
        );
        assert_eq!(
            settings_keys(CONFIG, &typo),
            Err(SourceError::Unresolved("OPTION_WHITELST".to_owned()))
        );
        assert_eq!(
            settings_keys(CONFIG, "pub const X: &str = \"x\";"),
            Err(SourceError::Missing(SETTINGS_LIST))
        );
        assert_eq!(
            settings_keys("", KEYS),
            Err(SourceError::Missing("pub mod keys"))
        );
    }
}
