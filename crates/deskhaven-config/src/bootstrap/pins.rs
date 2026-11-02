//! The raw `[settings]` table: pinned runtime settings, not interpreted here.
//!
//! Files may write pins as dotted keys (`"auth.session_ttl" = "30d"`) or as
//! nested tables; env always produces nested tables. [`NormalizedPins`]
//! rewrites file pins into nested tables before merging so every layer agrees
//! on one shape and figment precedence + `find_metadata` work per pin.
//!
//! Values from env, `_FILE` and CLI stay raw strings; the settings registry
//! parses them against each setting's type. Pins whose real name ends in
//! `_file` cannot be set via env (`<KEY>_FILE` always means indirection).
//!
//! Pin values may be sensitive (e.g. `smtp.password`), so `Debug` and
//! `Serialize` show keys only; read values through [`Pin::value`].

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;

use figment::value::{Dict, Map, Value};
use figment::{Error, Metadata, Profile, Provider};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Source;
use crate::types::REDACTED;

pub(crate) const SETTINGS_KEY: &str = "settings";

/// One pinned setting with its provenance.
#[derive(Clone, PartialEq)]
pub struct Pin {
    value: serde_json::Value,
    source: Source,
}

impl Pin {
    pub fn value(&self) -> &serde_json::Value {
        &self.value
    }

    pub fn source(&self) -> &Source {
        &self.source
    }
}

impl fmt::Debug for Pin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pin")
            .field("value", &REDACTED)
            .field("source", &self.source)
            .finish()
    }
}

/// Pinned settings keyed by dotted name, e.g. `auth.session_ttl`.
#[derive(Clone, Default, PartialEq)]
pub struct Pins(BTreeMap<String, Pin>);

impl Pins {
    pub fn get(&self, key: &str) -> Option<&Pin> {
        self.0.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Pin)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns a copy with each pin's source resolved by `source_of(key)`.
    pub(crate) fn with_sources(self, source_of: impl Fn(&str) -> Source) -> Self {
        let resolved = self.0.into_iter().map(|(key, pin)| {
            let source = source_of(&key);
            (key, Pin { source, ..pin })
        });
        Self(resolved.collect())
    }
}

impl fmt::Debug for Pins {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}

/// Keys with redacted values.
impl Serialize for Pins {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for key in self.0.keys() {
            map.serialize_entry(key, REDACTED)?;
        }
        map.end()
    }
}

/// Sources start as `Default`; the loader resolves them after merging.
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

fn flatten_into(out: &mut BTreeMap<String, Pin>, key: String, value: serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                flatten_into(out, format!("{key}.{k}"), v);
            }
        }
        value => {
            out.insert(
                key,
                Pin {
                    value,
                    source: Source::Default,
                },
            );
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
