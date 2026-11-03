//! Wire types of the settings endpoints. Timestamps are unix milliseconds (UTC).
//!
//! A sensitive setting's value is never on the wire: `value` and `default` are
//! `{"set": bool}` (the core's public form), history included.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use deskhaven_config::Source;
use deskhaven_config::settings::{self, SettingView as CoreView};
use deskhaven_core::settings::HistoryEntry;
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa::openapi::schema::{ObjectBuilder, Schema, SchemaType};

use crate::AdminApiError;

/// Any JSON value (OpenAPI 3.1 `{}`).
pub(crate) fn any_json() -> Schema {
    ObjectBuilder::new()
        .schema_type(SchemaType::AnyValue)
        .build()
        .into()
}

/// How the form edits a setting: the registry's value kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum SettingKind {
    #[serde(rename = "bool")]
    Bool,
    /// A whole number in `min..=max` (`zero_allowed`: or 0).
    #[serde(rename = "u32")]
    U32,
    /// A duration with units, e.g. `15m`, `7d` (`min..=max` as durations).
    #[serde(rename = "duration")]
    Duration,
    /// One of `variants`.
    #[serde(rename = "enum")]
    Enum,
    #[serde(rename = "string")]
    String,
    /// Write-only text: the API only says whether it is set.
    #[serde(rename = "secret")]
    Secret,
    /// A list of `host:port` items (at most `max_items`).
    #[serde(rename = "host_port_list")]
    HostPortList,
}

/// A bound of a number or duration range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum SchemaBound {
    Number(u64),
    /// A duration with units.
    Text(String),
}

/// Constraint of a text setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    Host,
    Email,
}

/// What a form needs to edit one setting (derived from the registry declaration).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SettingSchema {
    #[serde(rename = "type")]
    pub kind: SettingKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<SchemaBound>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<SchemaBound>,
    /// `0` is accepted besides `min..=max` ("off", "forever").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zero_allowed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<Vec<String>>,
    /// Longest text, in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<TextFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
}

/// The schemas of all settings, by key, built once from the registry's export.
fn schemas() -> &'static BTreeMap<String, SettingSchema> {
    static SCHEMAS: OnceLock<BTreeMap<String, SettingSchema>> = OnceLock::new();
    SCHEMAS.get_or_init(|| {
        let exported = settings::schema();
        let entries = exported["settings"].as_array().cloned().unwrap_or_default();
        entries
            .into_iter()
            .filter_map(|entry| {
                let key = entry["key"].as_str()?.to_owned();
                match serde_json::from_value(entry) {
                    Ok(schema) => Some((key, schema)),
                    Err(err) => {
                        tracing::error!(%key, error = %err, "setting schema is not exportable");
                        None
                    }
                }
            })
            .collect()
    })
}

/// Where the effective value comes from (05 §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceView {
    /// The built-in default.
    Default,
    /// Pinned by the configuration file; read-only here.
    File { path: String },
    /// Pinned by an environment variable (`<KEY>_FILE` included); read-only here.
    Env { var: String },
    /// Pinned by a command-line flag; read-only here.
    Cli,
    /// Stored in the database (the setting was modified in the console).
    Database {
        /// The stored row's version.
        version: i64,
        /// Account that wrote it; none for the system or when the account is gone.
        by: Option<i64>,
        by_username: Option<String>,
        /// When, unix milliseconds.
        at: i64,
    },
}

impl SourceView {
    fn new(source: &Source, names: &HashMap<i64, String>) -> Self {
        match source {
            Source::Default => Self::Default,
            Source::File { path } => Self::File {
                path: path.display().to_string(),
            },
            Source::Env { var } => Self::Env { var: var.clone() },
            Source::Cli => Self::Cli,
            Source::Database { version, by, at } => Self::Database {
                version: *version,
                by: *by,
                by_username: by.and_then(|id| names.get(&id).cloned()),
                at: *at,
            },
            // `Source` is non-exhaustive: a future source is shown as the default
            // rather than guessed at.
            _ => Self::Default,
        }
    }
}

/// One setting with its effective value.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct SettingView {
    /// Dotted key, e.g. `auth.session_ttl`.
    pub key: String,
    /// The first key segment: the form's group.
    pub category: String,
    pub description: String,
    /// The effective value in the form `PATCH` takes; `{"set": bool}` for a secret.
    #[schema(schema_with = any_json)]
    pub value: Value,
    #[schema(schema_with = any_json)]
    pub default: Value,
    pub source: SourceView,
    /// Fixed by the bootstrap configuration: not editable here.
    pub pinned: bool,
    /// A secret: write-only, never returned.
    pub sensitive: bool,
    /// Send back as `version` when changing or resetting the setting (0: nothing stored).
    pub version: i64,
    pub schema: SettingSchema,
}

impl SettingView {
    pub(crate) fn new(view: CoreView, names: &HashMap<i64, String>) -> Result<Self, AdminApiError> {
        let schema = schemas().get(view.key).cloned().ok_or_else(|| {
            AdminApiError::internal(format!("setting {} has no exportable schema", view.key))
        })?;
        let description = settings::lookup(view.key)
            .map(|s| s.description().to_owned())
            .unwrap_or_default();
        Ok(Self {
            key: view.key.to_owned(),
            category: view.category.to_owned(),
            description,
            value: view.value,
            default: view.default,
            source: SourceView::new(&view.source, names),
            pinned: view.pinned,
            sensitive: view.sensitive,
            version: view.version,
            schema,
        })
    }
}

/// The ids of the accounts behind the `Database` sources of `views`.
pub(crate) fn authors(views: &[CoreView]) -> Vec<i64> {
    views
        .iter()
        .filter_map(|v| match v.source {
            Source::Database { by, .. } => by,
            _ => None,
        })
        .collect()
}

/// One requested change: a new `value`, or `reset` to drop the stored value (exactly one).
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingChangeBody {
    #[garde(length(chars, min = 1, max = 128))]
    pub key: String,
    /// The `version` of the setting as last read; a different stored version is a 409.
    #[garde(range(min = 0))]
    pub version: i64,
    /// The new value: native JSON or its text form (`"30d"`, `true`, `["a:1"]`, a secret's
    /// text). Omit it with `reset`.
    #[serde(default)]
    #[schema(schema_with = any_json)]
    #[garde(skip)]
    pub value: Option<Value>,
    /// Delete the stored value, so the default applies again.
    #[serde(default)]
    #[garde(skip)]
    pub reset: bool,
}

/// The value may be a plaintext secret, so it is not shown (like `config::Change`).
impl std::fmt::Debug for SettingChangeBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingChangeBody")
            .field("key", &self.key)
            .field("version", &self.version)
            .field("reset", &self.reset)
            .finish_non_exhaustive()
    }
}

/// A batch of changes, applied together or not at all.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    #[garde(length(min = 1, max = 100), dive)]
    pub changes: Vec<SettingChangeBody>,
    /// Note stored in the history of every written key (at most 500 characters).
    #[garde(length(chars, max = 500))]
    pub reason: Option<String>,
}

/// One change of a setting.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct HistoryEntryView {
    /// The version the stored row had after this change.
    pub version: i64,
    /// Absent: nothing was stored before.
    #[schema(schema_with = any_json)]
    pub old_value: Option<Value>,
    /// The value that applied after this change (for a reset: the default).
    #[schema(schema_with = any_json)]
    pub new_value: Value,
    /// The stored value was dropped.
    pub reset: bool,
    pub updated_by: Option<i64>,
    pub updated_by_username: Option<String>,
    pub updated_at: i64,
    pub reason: Option<String>,
}

impl From<HistoryEntry> for HistoryEntryView {
    fn from(e: HistoryEntry) -> Self {
        Self {
            version: e.version,
            old_value: e.old_value,
            new_value: e.new_value,
            reset: e.reset,
            updated_by: e.updated_by,
            updated_by_username: e.updated_by_username,
            updated_at: e.updated_at,
            reason: e.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::*;

    #[test]
    fn debug_output_of_a_patch_hides_the_values() {
        let patch: SettingsPatch = serde_json::from_value(json!({
            "changes": [{ "key": "smtp.password", "version": 2, "value": "hunter2-secret" }],
            "reason": "rotate",
        }))
        .unwrap();

        let shown = format!("{patch:?}");

        assert!(!shown.contains("hunter2-secret"), "{shown}");
        assert!(
            shown.contains("smtp.password") && shown.contains("rotate"),
            "{shown}"
        );
    }

    #[test]
    fn every_registry_setting_has_an_exportable_schema() {
        for setting in settings::all() {
            assert!(schemas().contains_key(setting.key()), "{}", setting.key());
        }
    }

    #[test]
    fn pins_show_their_file_or_variable_and_database_sources_their_author() {
        let names = HashMap::from([(7, "alice".to_owned())]);
        let env = Source::Env {
            var: "DESKHAVEN_SETTINGS__AUTH__SESSION_TTL".to_owned(),
        };
        let file = Source::File {
            path: PathBuf::from("/etc/deskhaven/deskhaven.toml"),
        };
        let db = Source::Database {
            version: 3,
            by: Some(7),
            at: 1_700_000_000_000,
        };

        let shown: Vec<Value> = [env, file, Source::Cli, db, Source::Default]
            .iter()
            .map(|s| serde_json::to_value(SourceView::new(s, &names)).unwrap())
            .collect();

        assert_eq!(
            shown,
            [
                json!({ "kind": "env", "var": "DESKHAVEN_SETTINGS__AUTH__SESSION_TTL" }),
                json!({ "kind": "file", "path": "/etc/deskhaven/deskhaven.toml" }),
                json!({ "kind": "cli" }),
                json!({
                    "kind": "database", "version": 3, "by": 7,
                    "by_username": "alice", "at": 1_700_000_000_000_i64,
                }),
                json!({ "kind": "default" }),
            ]
        );
    }
}
