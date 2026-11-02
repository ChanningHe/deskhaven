//! Figment providers for each configuration layer and their identities.

use std::path::Path;

use figment::providers::{Format, Toml};
use figment::value::{Dict, Map};
use figment::{Error, Figment, Metadata, Profile, Provider};

use super::env::{ENV_PREFIX, EnvLayers, KEY_SEPARATOR};
use super::pins::NormalizedPins;

pub(crate) const ENV_LAYER: &str = "DESKHAVEN_* environment variable";
pub(crate) const ENV_FILE_LAYER: &str = "DESKHAVEN_*_FILE environment variable";
pub(crate) const CLI_LAYER: &str = "command-line flag";

/// Merges all layers: file < env < env `_FILE` < CLI. Defaults come from
/// serde, so a key absent from every layer has no metadata (= default).
pub(crate) fn build(file: Option<&Path>, env: EnvLayers, cli: Dict) -> Figment {
    let mut figment = Figment::new();
    if let Some(path) = file {
        figment = figment.merge(NormalizedPins(Toml::file_exact(path)));
    }
    figment
        .merge(Layer::new(env.plain, env_metadata(ENV_LAYER, "")))
        .merge(Layer::new(
            env.indirect,
            env_metadata(ENV_FILE_LAYER, "_FILE"),
        ))
        .merge(Layer::new(cli, Metadata::named(CLI_LAYER)))
}

/// Error messages name the variable (`DESKHAVEN_LISTEN__ADMIN`), not a path.
fn env_metadata(name: &'static str, suffix: &'static str) -> Metadata {
    Metadata::named(name).interpolater(move |_: &Profile, keys: &[&str]| {
        let joined = keys.join(KEY_SEPARATOR).to_ascii_uppercase();
        format!("{ENV_PREFIX}{joined}{suffix}")
    })
}

/// Precomputed values for the default profile, with a layer identity.
struct Layer {
    values: Dict,
    metadata: Metadata,
}

impl Layer {
    fn new(values: Dict, metadata: Metadata) -> Self {
        Self { values, metadata }
    }
}

impl Provider for Layer {
    fn metadata(&self) -> Metadata {
        self.metadata.clone()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        Ok(Profile::Default.collect(self.values.clone()))
    }
}
