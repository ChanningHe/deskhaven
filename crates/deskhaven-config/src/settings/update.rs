use std::collections::BTreeSet;
use std::fmt;

use serde_json::Value as Json;

use super::def::AnySetting;
use super::pins::PinSet;
use super::registry::lookup;
use super::rules;
use super::snapshot::Snapshot;
use super::value::Value;

/// `version` is the version the caller last saw (0 = never stored).
#[derive(Clone, PartialEq)]
pub struct Change {
    pub key: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reset {
    pub key: String,
    pub version: i64,
}

/// A change whose key exists, is neither pinned nor pin-only, and whose value is valid.
#[derive(Clone)]
pub struct Accepted {
    setting: &'static dyn AnySetting,
    value: Value,
    version: i64,
    reset: bool,
}

impl Accepted {
    /// The change drops the stored value; [`Accepted::value`] is the default that applies
    /// afterwards.
    pub fn is_reset(&self) -> bool {
        self.reset
    }

    pub fn key(&self) -> &'static str {
        self.setting.key()
    }

    pub fn setting(&self) -> &'static dyn AnySetting {
        self.setting
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

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
            .field("reset", &self.reset)
            .finish()
    }
}

/// Never contains the submitted value.
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Rejection {
    #[error("settings pinned by the bootstrap configuration: {}", keys.join(", "))]
    Pinned { keys: Vec<String> },
    #[error("invalid settings: {}", join(problems))]
    Invalid { problems: Vec<Problem> },
}

fn join(problems: &[Problem]) -> String {
    let rendered: Vec<String> = problems.iter().map(ToString::to_string).collect();
    rendered.join("; ")
}

/// Pinned keys take precedence over other problems; all problems are reported at once.
pub fn prepare(pins: &PinSet, changes: &[Change]) -> Result<Vec<Accepted>, Rejection> {
    prepare_batch(pins, changes, &[])
}

/// A key may appear once in the whole batch, as a change or as a reset.
pub fn prepare_batch(
    pins: &PinSet,
    changes: &[Change],
    resets: &[Reset],
) -> Result<Vec<Accepted>, Rejection> {
    let edits = changes
        .iter()
        .map(|c| (c.key.as_str(), c.version, Some(&c.value)))
        .chain(resets.iter().map(|r| (r.key.as_str(), r.version, None)));
    let mut seen = BTreeSet::new();
    let mut pinned = Vec::new();
    let mut problems = Vec::new();
    let mut accepted = Vec::with_capacity(changes.len() + resets.len());
    for (key, version, value) in edits {
        let Some(setting) = lookup(key) else {
            problems.push(Problem {
                key: echo_key(key),
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
        } else if pins.is_pinned(setting.key()) || setting.is_pin_only() {
            pinned.push(setting.key().to_owned());
        } else {
            match accept(setting, version, value) {
                Ok(change) => accepted.push(change),
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

fn accept(
    setting: &'static dyn AnySetting,
    version: i64,
    value: Option<&Json>,
) -> Result<Accepted, String> {
    let (value, reset) = match value {
        Some(raw) => (setting.coerce(raw)?, false),
        None => (setting.default_value(), true),
    };
    Ok(Accepted {
        setting,
        value,
        version,
        reset,
    })
}

const MAX_ECHOED_KEY: usize = 64;

/// An unknown, caller-supplied key made safe for logs and responses:
/// truncated and with control characters escaped.
pub(crate) fn echo_key(key: &str) -> String {
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

pub fn check_rules(candidate: &Snapshot, changed: &[&str]) -> Result<(), Rejection> {
    let problems: Vec<Problem> = rules::violations(candidate, Some(changed)).collect();
    match problems.is_empty() {
        true => Ok(()),
        false => Err(Rejection::Invalid { problems }),
    }
}
