//! Key paths derived from the schema itself, so there is no hand-kept list.

use std::collections::{BTreeMap, BTreeSet};

use figment::value::{Dict, Value};
use serde_json::Value as Json;

use super::error::ConfigError;
use super::pins::SETTINGS_KEY;
use super::schema::{Bootstrap, json_schema};

const SECRET_REF: &str = "#/$defs/Secret";
const DEFS_PREFIX: &str = "#/$defs/";

/// Every schema leaf (dotted path) with its default value, plus which leaves
/// are secrets (taken from the JSON Schema `$ref`s).
pub(crate) struct KnownKeys {
    defaults: BTreeMap<String, Json>,
    secrets: BTreeSet<String>,
}

impl KnownKeys {
    pub(crate) fn new() -> Result<Self, serde_json::Error> {
        let defaults = serde_json::to_value(Bootstrap::default())?;
        let schema = json_schema();
        let mut secrets = BTreeSet::new();
        collect_secrets(&schema, &schema, "", &mut secrets);
        Ok(Self {
            defaults: schema_leaves(defaults).into_iter().collect(),
            secrets,
        })
    }

    pub(crate) fn is_schema_leaf(&self, path: &str) -> bool {
        self.defaults.contains_key(path)
    }

    pub(crate) fn is_secret(&self, path: &str) -> bool {
        self.secrets.contains(path)
    }

    /// A schema leaf, or any well-formed key under `settings.` (pins).
    pub(crate) fn contains(&self, path: &str) -> bool {
        self.is_schema_leaf(path) || pin_key(path).is_some()
    }

    /// The single parser for raw strings from env, `_FILE` and CLI:
    /// string keys and pins stay verbatim (pins are typed later by the
    /// settings registry), lists accept `a,b` or `[a, b]`, other keys use
    /// figment's value syntax.
    pub(crate) fn typed_value(&self, path: &str, raw: &str) -> Value {
        match self.defaults.get(path) {
            None | Some(Json::String(_)) => Value::from(raw.to_owned()),
            Some(Json::Array(_)) => list_value(raw),
            Some(_) => parse_value(raw),
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

/// Walks `properties`, following `$defs` refs, collecting Secret-typed paths.
fn collect_secrets(root: &Json, node: &Json, prefix: &str, out: &mut BTreeSet<String>) {
    let Some(props) = node.get("properties").and_then(Json::as_object) else {
        return;
    };
    for (key, prop) in props {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match prop.get("$ref").and_then(Json::as_str) {
            Some(SECRET_REF) => {
                out.insert(path);
            }
            Some(reference) => {
                let name = reference.strip_prefix(DEFS_PREFIX).unwrap_or(reference);
                if let Some(def) = root.get("$defs").and_then(|defs| defs.get(name)) {
                    collect_secrets(root, def, &path, out);
                }
            }
            None => {}
        }
    }
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
        let expected =
            BTreeSet::from(["database.url".to_owned(), "security.master_key".to_owned()]);
        assert_eq!(keys.secrets, expected);
    }
}
