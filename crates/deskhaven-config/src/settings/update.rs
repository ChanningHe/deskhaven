//! Validation of an update batch, before and after it meets current state.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::Value as Json;

use super::def::AnySetting;
use super::pins::PinSet;
use super::registry::lookup;
use super::rules;
use super::snapshot::Snapshot;
use super::value::Value;

/// One requested change. `version` is the version the caller last saw
/// ([`SettingView::version`](super::SettingView), 0 = never stored).
#[derive(Clone, PartialEq)]
pub struct Change {
    pub key: String,
    /// Native or text JSON; may be a plaintext secret.
    pub value: Json,
    pub version: i64,
}

/// The value may be a plaintext secret, so it is not shown.
impl fmt::Debug for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Change")
            .field("key", &self.key)
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// A change whose key exists, is not pinned, and whose value is valid.
#[derive(Clone)]
pub struct Accepted {
    setting: &'static dyn AnySetting,
    value: Value,
    version: i64,
}

impl Accepted {
    pub fn key(&self) -> &'static str {
        self.setting.key()
    }

    pub fn setting(&self) -> &'static dyn AnySetting {
        self.setting
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn into_value(self) -> Value {
        self.value
    }

    /// The version the caller expects the stored row to have.
    pub fn expected_version(&self) -> i64 {
        self.version
    }
}

impl fmt::Debug for Accepted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Accepted")
            .field("key", &self.key())
            .field("value", &self.value)
            .field("version", &self.version)
            .finish()
    }
}

/// Why one key was rejected. Never contains the submitted value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub key: String,
    pub reason: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.key, self.reason)
    }
}

/// A batch that must not be applied (nothing is written).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    /// Keys fixed by the bootstrap configuration (API: 409 `setting_pinned`).
    #[error("settings pinned by the bootstrap configuration: {}", keys.join(", "))]
    Pinned { keys: Vec<String> },
    /// Unknown keys, duplicates, invalid values or violated cross-key rules.
    #[error("invalid settings: {}", join(problems))]
    Invalid { problems: Vec<Problem> },
}

fn join(problems: &[Problem]) -> String {
    let rendered: Vec<String> = problems.iter().map(ToString::to_string).collect();
    rendered.join("; ")
}

/// Checks everything that does not depend on current state. Pinned keys
/// take precedence over other problems; all problems are reported at once.
pub fn prepare(pins: &PinSet, changes: &[Change]) -> Result<Vec<Accepted>, Rejection> {
    let mut seen = BTreeSet::new();
    let mut pinned = Vec::new();
    let mut problems = Vec::new();
    let mut accepted = Vec::with_capacity(changes.len());
    for change in changes {
        let Some(setting) = lookup(&change.key) else {
            problems.push(Problem {
                key: echo_key(&change.key),
                reason: "unknown setting".to_owned(),
            });
            continue;
        };
        let problem = |reason: &str| Problem {
            key: setting.key().to_owned(),
            reason: reason.to_owned(),
        };
        if !seen.insert(setting.key()) {
            problems.push(problem("set more than once in one update"));
        } else if pins.is_pinned(setting.key()) {
            pinned.push(setting.key().to_owned());
        } else {
            match setting.coerce(&change.value) {
                Ok(value) => accepted.push(Accepted {
                    setting,
                    value,
                    version: change.version,
                }),
                Err(reason) => problems.push(problem(&reason)),
            }
        }
    }
    if !pinned.is_empty() {
        return Err(Rejection::Pinned { keys: pinned });
    }
    match problems.is_empty() {
        true => Ok(accepted),
        false => Err(Rejection::Invalid { problems }),
    }
}

/// Longest unknown key echoed back in a [`Problem`].
const MAX_ECHOED_KEY: usize = 64;

/// An unknown, caller-supplied key made safe for logs and responses:
/// truncated and with control characters escaped.
fn echo_key(key: &str) -> String {
    let mut echoed: String = key
        .chars()
        .take(MAX_ECHOED_KEY)
        .flat_map(char::escape_debug)
        .collect();
    if key.chars().nth(MAX_ECHOED_KEY).is_some() {
        echoed.push('…');
    }
    echoed
}

/// Checks the cross-key rules that involve a `changed` key on the
/// candidate snapshot (current state with the batch applied).
pub fn check_rules(candidate: &Snapshot, changed: &[&str]) -> Result<(), Rejection> {
    let problems: Vec<Problem> = rules::violations(candidate, Some(changed)).collect();
    match problems.is_empty() {
        true => Ok(()),
        false => Err(Rejection::Invalid { problems }),
    }
}
