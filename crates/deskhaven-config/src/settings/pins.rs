//! Validated pins: settings fixed by the bootstrap configuration (05-config §3.4).

use std::collections::BTreeMap;
use std::fmt;

use super::registry::lookup;
use super::update::echo_key;
use super::value::Value;
use crate::Source;
use crate::bootstrap::Pins;

/// Pinned settings, coerced against the registry. A pinned key is read-only
/// at runtime: its effective value is the pin, any stored value is ignored.
#[derive(Debug, Clone, Default)]
pub struct PinSet {
    pins: BTreeMap<&'static str, Pinned>,
}

#[derive(Debug, Clone)]
pub(crate) struct Pinned {
    pub(crate) value: Value,
    pub(crate) source: Source,
}

impl PinSet {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Coerces every raw pin (TOML-native or text) against its setting.
    /// Unknown keys and invalid values are all reported together; reasons
    /// name the key and source, never the value.
    pub fn from_bootstrap(raw: &Pins) -> Result<Self, PinError> {
        let mut pins = BTreeMap::new();
        let mut problems = Vec::new();
        for (key, pin) in raw.iter() {
            let problem = |reason: String| PinProblem {
                key: echo_key(key),
                source: pin.source().clone(),
                reason,
            };
            let Some(setting) = lookup(key) else {
                problems.push(problem("unknown setting".to_owned()));
                continue;
            };
            match setting.coerce(pin.value()) {
                Ok(value) => {
                    let source = pin.source().clone();
                    pins.insert(setting.key(), Pinned { value, source });
                }
                Err(reason) => problems.push(problem(reason)),
            }
        }
        match problems.is_empty() {
            true => Ok(Self { pins }),
            false => Err(PinError { problems }),
        }
    }

    pub fn is_pinned(&self, key: &str) -> bool {
        self.pins.contains_key(key)
    }

    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Pinned> {
        self.pins.get(key)
    }
}

/// One rejected pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinProblem {
    pub key: String,
    pub source: Source,
    pub reason: String,
}

impl fmt::Display for PinProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "settings.{} (from {}): {}",
            self.key, self.source, self.reason
        )
    }
}

/// Startup error: some pins are unknown or invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid settings pins: {}", join(problems))]
pub struct PinError {
    pub problems: Vec<PinProblem>,
}

fn join(problems: &[PinProblem]) -> String {
    let rendered: Vec<String> = problems.iter().map(ToString::to_string).collect();
    rendered.join("; ")
}
