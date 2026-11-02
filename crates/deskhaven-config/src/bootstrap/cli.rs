//! Command-line overrides: the highest-precedence layer.

use std::collections::BTreeMap;

use figment::value::Dict;

use super::error::ConfigError;
use super::keys::{KnownKeys, dict_from};

/// Values given as CLI flags, keyed by dotted path (e.g. `data_dir`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOverrides(BTreeMap<String, String>);

impl CliOverrides {
    /// Returns a copy with `key` set to `value`.
    #[must_use]
    pub fn with(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let mut map = self.0;
        map.insert(key.into(), value.into());
        Self(map)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Validates keys (unknown = error, unlike env) and builds the layer.
    pub(crate) fn to_dict(&self, known: &KnownKeys) -> Result<Dict, ConfigError> {
        let mut values = Vec::with_capacity(self.0.len());
        for (key, raw) in &self.0 {
            if !known.contains(key) {
                return Err(ConfigError::UnknownCliKey { key: key.clone() });
            }
            values.push((key.clone(), known.typed_value(key, raw)));
        }
        dict_from(values)
    }
}
