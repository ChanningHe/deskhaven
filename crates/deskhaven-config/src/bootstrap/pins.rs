//! The raw `[settings]` table: pinned runtime settings, not interpreted here.
//!
//! Files may write pins as dotted keys (`"auth.session_ttl" = "30d"`) or as
//! nested tables; env always produces nested tables. [`NormalizedPins`]
//! rewrites file pins into nested tables before merging so every layer agrees
//! on one shape and figment precedence + `find_metadata` work per pin.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use figment::value::{Dict, Map, Value};
use figment::{Error, Metadata, Profile, Provider};
use serde::{Deserialize, Deserializer, Serialize};

pub(crate) const SETTINGS_KEY: &str = "settings";

/// Pinned settings flattened to dotted keys, e.g. `auth.session_ttl`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Pins(BTreeMap<String, serde_json::Value>);

impl Pins {
    pub fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.0.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &serde_json::Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for Pins {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let nested = BTreeMap::<String, serde_json::Value>::deserialize(d)?;
        let mut flat = BTreeMap::new();
        for (key, value) in nested {
            flatten_into(&mut flat, key, value);
        }
        Ok(Self(flat))
    }
}

fn flatten_into(
    out: &mut BTreeMap<String, serde_json::Value>,
    key: String,
    value: serde_json::Value,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                flatten_into(out, format!("{key}.{k}"), v);
            }
        }
        leaf => {
            out.insert(key, leaf);
        }
    }
}

/// Wraps a file provider and nests dotted keys inside its `[settings]` table.
pub(crate) struct NormalizedPins<P>(pub P);

impl<P: Provider> Provider for NormalizedPins<P> {
    /// Errors name the plain dotted key (`listen.typo`, not `default.listen.typo`).
    fn metadata(&self) -> Metadata {
        self.0
            .metadata()
            .interpolater(|_: &Profile, keys: &[&str]| keys.join("."))
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        let mut out = Map::new();
        for (profile, dict) in self.0.data()? {
            out.insert(profile, normalize_root(dict).map_err(Error::from)?);
        }
        Ok(out)
    }
}

fn normalize_root(mut root: Dict) -> Result<Dict, String> {
    let normalized = match root.remove(SETTINGS_KEY) {
        Some(Value::Dict(tag, table)) => Value::Dict(tag, nest_dotted(table, "")?),
        // A non-table `settings` is left for extraction to report as a type error.
        Some(other) => other,
        None => return Ok(root),
    };
    root.insert(SETTINGS_KEY.to_owned(), normalized);
    Ok(root)
}

fn nest_dotted(table: Dict, prefix: &str) -> Result<Dict, String> {
    let mut out = Dict::new();
    for (key, value) in table {
        let path = join(prefix, &key);
        if key.split('.').any(str::is_empty) {
            return Err(format!("invalid settings key `{path}`: empty segment"));
        }
        let value = match value {
            Value::Dict(tag, inner) => Value::Dict(tag, nest_dotted(inner, &path)?),
            leaf => leaf,
        };
        let nested = key.rsplit('.').fold(value, |acc, seg| {
            Value::from(Dict::from([(seg.to_owned(), acc)]))
        });
        if let Value::Dict(_, single) = nested {
            merge_strict(&mut out, single, prefix)?;
        }
    }
    Ok(out)
}

/// Deep-merges `src` into `target`, rejecting any key defined twice.
fn merge_strict(target: &mut Dict, src: Dict, prefix: &str) -> Result<(), String> {
    for (key, value) in src {
        let path = join(prefix, &key);
        match target.entry(key) {
            Entry::Vacant(slot) => {
                slot.insert(value);
            }
            Entry::Occupied(mut slot) => match (slot.get_mut(), value) {
                (Value::Dict(_, existing), Value::Dict(_, incoming)) => {
                    merge_strict(existing, incoming, &path)?;
                }
                _ => {
                    let msg = format!("settings key `{path}` is defined more than once");
                    return Err(msg);
                }
            },
        }
    }
    Ok(())
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}
