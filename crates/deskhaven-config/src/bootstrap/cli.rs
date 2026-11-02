//! Command-line overrides: the highest-precedence layer.

use std::collections::BTreeMap;
use std::fmt;

use figment::value::Dict;

use super::error::ConfigError;
use super::keys::{KnownKeys, dict_from};
use crate::types::REDACTED;

/// Values given as CLI flags, keyed by dotted path (e.g. `data_dir`).
/// `Debug` shows keys only, so `LoadOptions`' derived `Debug` is safe too.
#[derive(Clone, Default, PartialEq, Eq)]
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

impl fmt::Debug for CliOverrides {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|key| (key, REDACTED)))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::LoadOptions;

    #[test]
    fn debug_hides_values() {
        let cli = CliOverrides::default().with("settings.smtp.password", "HUNTER2");
        let options = LoadOptions {
            cli,
            ..LoadOptions::default()
        };
        let debug = format!("{options:?}");
        assert!(debug.contains("settings.smtp.password") && !debug.contains("HUNTER2"));
    }
}
