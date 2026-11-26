use deskhaven_config::settings::Snapshot;
use serde_json::Value as Json;

/// A sensitive value is `{"set": bool}`, never the secret or its ciphertext.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingChange {
    pub key: &'static str,
    pub before: Json,
    pub after: Json,
    pub reset: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppliedChanges {
    pub changes: Vec<SettingChange>,
    pub reason: Option<String>,
}

impl AppliedChanges {
    pub(super) fn between(
        before: &Snapshot,
        after: &Snapshot,
        written: &[(&'static str, bool)],
        reason: Option<&str>,
    ) -> Self {
        let value = |snapshot: &Snapshot, key: &str, usable_only: bool| {
            snapshot
                .entry(key)
                .filter(|entry| !(usable_only && entry.is_fallback()))
                .map_or(Json::Null, |entry| entry.value().to_json())
        };
        let changes = written
            .iter()
            .map(|&(key, reset)| SettingChange {
                key,
                before: value(before, key, true),
                after: value(after, key, false),
                reset,
            })
            .collect();
        Self {
            changes,
            reason: reason.map(str::to_owned),
        }
    }
}
