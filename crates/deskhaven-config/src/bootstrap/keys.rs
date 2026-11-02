//! Key paths derived from the schema itself, so there is no hand-kept list.

use std::collections::BTreeMap;

use figment::value::{Dict, Value};
use serde_json::Value as Json;

use super::error::ConfigError;
use super::pins::SETTINGS_KEY;
use super::schema::Bootstrap;

/// Every schema leaf (dotted path) with its default value.
pub(crate) struct KnownKeys(BTreeMap<String, Json>);

impl KnownKeys {
    pub(crate) fn new() -> Result<Self, serde_json::Error> {
        let defaults = serde_json::to_value(Bootstrap::default())?;
        Ok(Self(schema_leaves(defaults).into_iter().collect()))
    }

    /// A schema leaf, or any well-formed key under `settings.` (pins).
    pub(crate) fn contains(&self, path: &str) -> bool {
        self.0.contains_key(path) || pin_key(path).is_some()
    }

    /// Converts a raw string (env / file / CLI) into a figment value. String
    /// keys keep the text verbatim; others use figment's env value syntax.
    pub(crate) fn typed_value(&self, path: &str, raw: &str) -> Value {
        match self.0.get(path) {
            Some(Json::String(_)) => Value::from(raw.to_owned()),
            _ => {
                let Ok(value) = raw.parse::<Value>();
                value
            }
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
    }
}
