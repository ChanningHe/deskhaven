//! Key paths derived from the schema itself, so there is no hand-kept list.

use std::collections::BTreeSet;

use figment::value::{Dict, Value};
use serde_json::Value as Json;

use super::error::ConfigError;
use super::pins::SETTINGS_KEY;
use super::schema::{Bootstrap, json_schema};
use super::schema_facts::SchemaFacts;

/// Every schema leaf (dotted path, from the serialized defaults) plus type
/// facts from the JSON Schema.
pub(crate) struct KnownKeys {
    leaves: BTreeSet<String>,
    facts: SchemaFacts,
}

impl KnownKeys {
    pub(crate) fn new() -> Result<Self, serde_json::Error> {
        let defaults = serde_json::to_value(Bootstrap::default())?;
        Ok(Self {
            leaves: schema_leaves(defaults)
                .into_iter()
                .map(|(k, _)| k)
                .collect(),
            facts: SchemaFacts::from_schema(&json_schema()),
        })
    }

    pub(crate) fn is_schema_leaf(&self, path: &str) -> bool {
        self.leaves.contains(path)
    }

    pub(crate) fn is_secret(&self, path: &str) -> bool {
        self.facts.is_secret(path)
    }

    /// A schema leaf, or any well-formed key under `settings.` (pins).
    pub(crate) fn contains(&self, path: &str) -> bool {
        self.is_schema_leaf(path) || pin_key(path).is_some()
    }

    /// The single parser for raw strings from env, `_FILE` and CLI. Shape
    /// comes from the schema `type` (so `Option<Vec<_>>` is still a list):
    /// lists accept `a,b` or `[a, b]`; string keys and pins stay verbatim
    /// (pins are typed later by the settings registry); other keys use
    /// figment's value syntax.
    pub(crate) fn typed_value(&self, path: &str, raw: &str) -> Value {
        if !self.is_schema_leaf(path) || self.facts.is_text(path) && !self.facts.is_list(path) {
            Value::from(raw.to_owned())
        } else if self.facts.is_list(path) {
            list_value(raw)
        } else {
            parse_value(raw)
        }
    }
}

fn parse_value(raw: &str) -> Value {
    let Ok(value) = raw.parse::<Value>();
    value
}

fn list_value(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        return parse_value(trimmed);
    }
    let items: Vec<Value> = trimmed
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| Value::from(item.to_owned()))
        .collect();
    Value::from(items)
}

/// Builds a nested dict from `(dotted path, value)` pairs. A path that is both
/// a value and a table (`a` and `a.b`) is rejected rather than silently dropped.
pub(crate) fn dict_from(entries: Vec<(String, Value)>) -> Result<Dict, ConfigError> {
    let mut root = Dict::new();
    for (path, value) in entries {
        insert_path(&mut root, &path, value).map_err(|()| ConfigError::Invalid {
            problems: vec![format!(
                "`{path}` conflicts with another key on the same path"
            )],
        })?;
    }
    Ok(root)
}

fn insert_path(dict: &mut Dict, path: &str, value: Value) -> Result<(), ()> {
    let Some((head, rest)) = path.split_once('.') else {
        return match dict.insert(path.to_owned(), value) {
            None => Ok(()),
            Some(_) => Err(()),
        };
    };
    let slot = dict
        .entry(head.to_owned())
        .or_insert_with(|| Value::from(Dict::new()));
    match slot {
        Value::Dict(_, child) => insert_path(child, rest, value),
        _ => Err(()),
    }
}

/// The settings key of a pin path (`settings.auth.ttl` -> `auth.ttl`).
fn pin_key(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(SETTINGS_KEY)?.strip_prefix('.')?;
    (!rest.split('.').any(str::is_empty)).then_some(rest)
}

/// Leaves of a serialized `Bootstrap`, excluding the `settings` subtree.
pub(crate) fn schema_leaves(root: Json) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    if let Json::Object(mut map) = root {
        map.remove(SETTINGS_KEY);
        for (key, value) in map {
            walk(key, value, &mut out);
        }
    }
    out
}

fn walk(path: String, value: Json, out: &mut Vec<(String, Json)>) {
    match value {
        Json::Object(map) if !map.is_empty() => {
            for (key, child) in map {
                walk(format!("{path}.{key}"), child, out);
            }
        }
        leaf => out.push((path, leaf)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_schema_leaves_and_pins_only() {
        let keys = KnownKeys::new().unwrap();
        assert!(keys.contains("listen.admin"));
        assert!(keys.contains("security.trusted_proxies"));
        assert!(keys.contains("settings.auth.session_ttl"));
        assert!(!keys.contains("listen"));
        assert!(!keys.contains("settings"));
        assert!(!keys.contains("settings..x"));
        assert!(!keys.contains("bogus"));
    }

    #[test]
    fn string_keys_stay_verbatim() {
        let keys = KnownKeys::new().unwrap();
        assert_eq!(
            keys.typed_value("log.level", "123"),
            Value::from("123".to_owned())
        );
        assert_eq!(
            keys.typed_value("signal.port", "123"),
            "123".parse::<Value>().unwrap()
        );
        let pin = keys.typed_value("settings.smtp.password", "0042");
        assert_eq!(pin, Value::from("0042".to_owned()));
    }

    #[test]
    fn secrets_come_from_schema() {
        let keys = KnownKeys::new().unwrap();
        assert!(keys.is_secret("database.url") && keys.is_secret("security.master_key"));
        assert!(!keys.is_secret("public_url") && !keys.is_secret("database.max_connections"));
    }

    #[test]
    fn list_detection_uses_schema_type() {
        let keys = KnownKeys::new().unwrap();
        let expected = Value::from(vec![Value::from("10.0.0.0/8".to_owned())]);
        assert_eq!(
            keys.typed_value("security.trusted_proxies", "10.0.0.0/8"),
            expected
        );
    }
}
