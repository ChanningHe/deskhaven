//! Immutable snapshot of every effective setting: pin > database > default.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;
use serde_json::Value as Json;

use super::def::{AnySetting, Setting};
use super::kind::SettingType;
use super::pins::PinSet;
use super::registry::{all, lookup};
use super::rules;
use super::value::Value;
use crate::Source;

/// One effective value with its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    value: Value,
    source: Source,
    pinned: bool,
}

impl Entry {
    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    /// Optimistic-lock version of the stored row; 0 when nothing is stored.
    pub fn version(&self) -> i64 {
        match self.source {
            Source::Database { version, .. } => version,
            _ => 0,
        }
    }
}

/// A row as read from storage, with sensitive values already decrypted.
#[derive(Clone, PartialEq)]
pub struct Stored {
    pub key: String,
    pub value: Json,
    pub version: i64,
    pub by: Option<i64>,
    /// Unix milliseconds.
    pub at: i64,
}

/// Values may be decrypted secrets, so they are not shown.
impl fmt::Debug for Stored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Stored")
            .field("key", &self.key)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// A validated value replacing the effective value of `key`.
#[derive(Debug, Clone)]
pub struct Update {
    pub key: &'static str,
    pub value: Value,
    pub source: Source,
}

/// A stored value that was ignored, or a cross-key rule that does not hold.
/// Never contains values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadWarning {
    pub key: String,
    pub problem: String,
}

impl fmt::Display for LoadWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "setting `{}`: {}", self.key, self.problem)
    }
}

/// Read-API form of one setting (`GET /admin/api/settings`, 05-config §3.5).
/// Sensitive values are `{"set": bool}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingView {
    pub key: &'static str,
    pub category: &'static str,
    pub value: Json,
    pub default: Json,
    pub source: Source,
    pub pinned: bool,
    pub sensitive: bool,
    pub version: i64,
}

/// Every setting's effective value. Immutable; a change builds a new one.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    entries: BTreeMap<&'static str, Entry>,
}

impl Snapshot {
    /// Defaults only: no pins, nothing stored.
    pub fn defaults() -> Self {
        Self::build(&PinSet::empty(), Vec::new()).0
    }

    /// Resolves pin > stored > default for every declared setting. Unknown
    /// or invalid stored rows are ignored (the default applies) and reported,
    /// as are cross-key rules that do not hold: a bad row must not keep the
    /// server, and with it the settings UI, from starting.
    pub fn build(
        pins: &PinSet,
        stored: impl IntoIterator<Item = Stored>,
    ) -> (Self, Vec<LoadWarning>) {
        let mut warnings = Vec::new();
        let mut from_db = BTreeMap::new();
        for row in stored {
            match decode(row) {
                Ok((key, entry)) => {
                    from_db.insert(key, entry);
                }
                Err(warning) => warnings.push(warning),
            }
        }
        let entries = all()
            .iter()
            .map(|setting| {
                let key = setting.key();
                let entry = pinned(pins, key)
                    .or_else(|| from_db.remove(key))
                    .unwrap_or_else(|| default_entry(*setting));
                (key, entry)
            })
            .collect();
        let snapshot = Self { entries };
        warnings.extend(rules::violations(&snapshot, None).map(|p| LoadWarning {
            key: p.key,
            problem: p.reason,
        }));
        (snapshot, warnings)
    }

    /// The typed effective value of `setting`.
    pub fn get<T: SettingType>(&self, setting: &Setting<T>) -> T {
        // Entries are built only from values coerced by the same declaration,
        // so the fallback is unreachable; it avoids a panic path.
        self.entries
            .get(setting.key())
            .and_then(|entry| T::from_value(&entry.value))
            .unwrap_or_else(|| setting.default())
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.get(key)
    }

    /// A copy with `updates` applied. Pinned keys keep their pin.
    pub fn with_updates(&self, updates: impl IntoIterator<Item = Update>) -> Self {
        let mut entries = self.entries.clone();
        for update in updates {
            if let Some(entry) = entries.get_mut(update.key).filter(|e| !e.pinned) {
                *entry = Entry {
                    value: update.value,
                    source: update.source,
                    pinned: false,
                };
            }
        }
        Self { entries }
    }

    pub fn view(&self, key: &str) -> Option<SettingView> {
        let setting = lookup(key)?;
        let entry = self.entries.get(setting.key())?;
        Some(SettingView {
            key: setting.key(),
            category: setting.category(),
            value: entry.value.to_json(),
            default: setting.default_value().to_json(),
            source: entry.source.clone(),
            pinned: entry.pinned,
            sensitive: setting.is_sensitive(),
            version: entry.version(),
        })
    }

    /// Every setting, in declaration order.
    pub fn views(&self) -> Vec<SettingView> {
        all().iter().filter_map(|s| self.view(s.key())).collect()
    }
}

fn pinned(pins: &PinSet, key: &str) -> Option<Entry> {
    pins.get(key).map(|pin| Entry {
        value: pin.value.clone(),
        source: pin.source.clone(),
        pinned: true,
    })
}

fn default_entry(setting: &dyn AnySetting) -> Entry {
    Entry {
        value: setting.default_value(),
        source: Source::Default,
        pinned: false,
    }
}

fn decode(row: Stored) -> Result<(&'static str, Entry), LoadWarning> {
    let warning = |problem: String| LoadWarning {
        key: row.key.clone(),
        problem,
    };
    let setting = lookup(&row.key)
        .ok_or_else(|| warning("stored value for an unknown setting ignored".to_owned()))?;
    let value = setting.coerce(&row.value).map_err(|reason| {
        warning(format!(
            "stored value ignored ({reason}); using the default"
        ))
    })?;
    let source = Source::Database {
        version: row.version,
        by: row.by,
        at: row.at,
    };
    Ok((
        setting.key(),
        Entry {
            value,
            source,
            pinned: false,
        },
    ))
}
