//! What a batch changed, as the audit guard of the same transaction sees it.

use deskhaven_config::settings::Snapshot;
use serde_json::Value as Json;

/// One written key. Values are the public JSON form: a sensitive value is
/// `{"set": bool}`, never the secret or its ciphertext.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingChange {
    pub key: &'static str,
    /// The effective value before the batch (`null` when the stored row was unusable).
    pub before: Json,
    /// The effective value after it (the default, for a reset).
    pub after: Json,
    /// The stored row was dropped, so the default applies again.
    pub reset: bool,
}

/// The keys one committed batch wrote, in request order.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedChanges {
    pub changes: Vec<SettingChange>,
}

impl AppliedChanges {
    /// The keys of `written` as they were in `before` and are in `after`.
    pub(super) fn between(
        before: &Snapshot,
        after: &Snapshot,
        written: &[(&'static str, bool)],
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
        Self { changes }
    }
}
