//! Effective view: every key with its (redacted) value and provenance.

use figment::Figment;
use serde::Serialize;

use super::env::EnvPlan;
use super::keys::schema_leaves;
use super::layers::{CLI_LAYER, ENV_FILE_LAYER, ENV_LAYER};
use super::pins::SETTINGS_KEY;
use super::schema::Bootstrap;
use crate::Source;
use crate::types::REDACTED;

/// One effective key. Secrets (and, until the settings registry knows which
/// pins are sensitive, all pin values) are rendered as `<redacted>`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EffectiveEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub source: Source,
}

/// Schema leaves (sources from `figment`), then pins (sources already resolved).
pub(crate) fn collect(
    figment: &Figment,
    config: &Bootstrap,
    env: &EnvPlan,
) -> Result<Vec<EffectiveEntry>, serde_json::Error> {
    let schema = schema_leaves(serde_json::to_value(config)?)
        .into_iter()
        .map(|(key, value)| {
            let source = source_of(figment, env, &key);
            EffectiveEntry { key, value, source }
        });
    let pins = config.settings.iter().map(|(key, pin)| EffectiveEntry {
        key: format!("{SETTINGS_KEY}.{key}"),
        value: serde_json::Value::from(REDACTED),
        source: pin.source().clone(),
    });
    Ok(schema.chain(pins).collect())
}

/// Maps the metadata of the layer that won `path` to a [`Source`].
pub(crate) fn source_of(figment: &Figment, env: &EnvPlan, path: &str) -> Source {
    let Some(metadata) = figment.find_metadata(path) else {
        return Source::Default;
    };
    match metadata.name.as_ref() {
        ENV_LAYER | ENV_FILE_LAYER => {
            let var = env.var_for(path).map(str::to_owned).unwrap_or_else(|| {
                metadata.interpolate(
                    &figment::Profile::Default,
                    &path.split('.').collect::<Vec<_>>(),
                )
            });
            Source::Env { var }
        }
        CLI_LAYER => Source::Cli,
        _ => match &metadata.source {
            Some(figment::Source::File(path)) => Source::File { path: path.clone() },
            _ => Source::Default,
        },
    }
}
