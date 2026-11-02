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
use super::update::Accepted;
use super::value::Value;
use crate::Source;

/// One effective value with its provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    value: Value,
    source: Source,
    pinned: bool,
    version: i64,
    fallback: bool,
}

impl Entry {
    /// A stored row exists but could not be used; the value is the
    /// setting's bad-row fallback (strict for security-relevant settings).
    pub fn is_fallback(&self) -> bool {
        self.fallback
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn pinned(&self) -> bool {
        self.pinned
    }

    /// Optimistic-lock version of the stored row; 0 when no row exists.
    /// Also set when the row was ignored (unreadable or invalid), so a
    /// client can overwrite it.
    pub fn version(&self) -> i64 {
        self.version
    }
}

/// A row as read from storage, with sensitive values already decrypted.
#[derive(Clone, PartialEq)]
pub struct Stored {
    pub key: String,
    /// `None`: the row exists but could not be read (the caller reported
    /// why); the default applies but the version is kept.
    pub value: Option<Json>,
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
    /// Send back as [`Change::version`](super::Change) when updating.
    pub version: i64,
}

/// Every setting's effective value. Immutable; a change builds a new one.
/// Only [`Snapshot::build`] and [`Snapshot::with_accepted`] create entries,
/// and both take values coerced by the setting's own declaration.
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
    /// or invalid stored rows are ignored (the setting's bad-row fallback
    /// applies, the row's version is kept) and reported, as are cross-key
    /// rules that do not hold: a bad row must not keep the server, and with
    /// it the settings UI, from starting.
    pub fn build(
        pins: &PinSet,
        stored: impl IntoIterator<Item = Stored>,
    ) -> (Self, Vec<LoadWarning>) {
        let mut warnings = Vec::new();
        let mut rows = BTreeMap::new();
        for row in stored {
            match decode(row) {
                Ok((key, row)) => {
                    rows.insert(key, row);
                }
                Err(warning) => warnings.push(warning),
            }
        }
        let entries = all()
            .iter()
            .map(|setting| {
                (
                    setting.key(),
                    resolve(*setting, pins, rows.remove(setting.key()), &mut warnings),
                )
            })
            .collect();
        let snapshot = Self { entries };
        warnings.extend(snapshot.rule_warnings());
        (snapshot, warnings)
    }

    /// The typed effective value of `setting`.
    pub fn get<T: SettingType>(&self, setting: &Setting<T>) -> T {
        // Entries hold only values coerced by the same declaration, so the
        // fallback is unreachable; it avoids a panic path.
        self.entries
            .get(setting.key())
            .and_then(|entry| T::from_value(&entry.value))
            .unwrap_or_else(|| setting.default())
    }

    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.get(key)
    }

    /// A copy with `accepted` applied as stored at version
    /// `expected + 1` by `by` at `at` (unix ms). Pinned keys keep their pin.
    pub fn with_accepted(&self, accepted: &[Accepted], by: Option<i64>, at: i64) -> Self {
        let mut entries = self.entries.clone();
        for change in accepted {
            let Some(entry) = entries.get_mut(change.key()).filter(|e| !e.pinned) else {
                continue;
            };
            let version = change.expected_version() + 1;
            *entry = Entry {
                value: change.value().clone(),
                source: Source::Database { version, by, at },
                pinned: false,
                version,
                fallback: false,
            };
        }
        Self { entries }
    }

    /// Cross-key rules that do not hold (also part of [`Snapshot::build`]'s warnings).
    pub fn rule_warnings(&self) -> Vec<LoadWarning> {
        rules::violations(self, None)
            .map(|p| LoadWarning {
                key: p.key,
                problem: p.reason,
            })
            .collect()
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
            version: entry.version,
        })
    }

    /// Every setting, in declaration order.
    pub fn views(&self) -> Vec<SettingView> {
        all().iter().filter_map(|s| self.view(s.key())).collect()
    }
}

/// Pin, else a readable valid row, else the bad-row fallback (row exists
/// but is unusable; keeps the row version), else the default.
fn resolve(
    setting: &dyn AnySetting,
    pins: &PinSet,
    row: Option<Stored>,
    warnings: &mut Vec<LoadWarning>,
) -> Entry {
    let version = row.as_ref().map_or(0, |r| r.version);
    let entry = |value, source, pinned, fallback| Entry {
        value,
        source,
        pinned,
        version,
        fallback,
    };
    if let Some(pin) = pins.get(setting.key()) {
        return entry(pin.value.clone(), pin.source.clone(), true, false);
    }
    let Some(row) = row else {
        return entry(setting.default_value(), Source::Default, false, false);
    };
    match row.value.as_ref().map(|raw| setting.coerce(raw)) {
        Some(Ok(value)) => {
            let source = Source::Database {
                version: row.version,
                by: row.by,
                at: row.at,
            };
            entry(value, source, false, false)
        }
        Some(Err(reason)) => {
            warnings.push(LoadWarning {
                key: row.key,
                problem: format!("stored value ignored ({reason})"),
            });
            entry(setting.bad_row_value(), Source::Default, false, true)
        }
        // Unreadable; the caller already reported why.
        None => entry(setting.bad_row_value(), Source::Default, false, true),
    }
}

/// Rows of unknown settings are reported and dropped.
fn decode(row: Stored) -> Result<(&'static str, Stored), LoadWarning> {
    match lookup(&row.key) {
        Some(setting) => Ok((setting.key(), row)),
        None => Err(LoadWarning {
            key: row.key,
            problem: "stored value for an unknown setting ignored".to_owned(),
        }),
    }
}
