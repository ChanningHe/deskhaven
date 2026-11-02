//! Figment providers for each configuration layer and their identities.

use figment::providers::{Env, Format, Toml};
use figment::value::{Dict, Map};
use figment::{Error, Figment, Metadata, Profile, Provider};

use super::env::{ENV_PREFIX, EnvPlan, KEY_SEPARATOR};
use super::pins::NormalizedPins;
use std::path::Path;

pub(crate) const ENV_LAYER: &str = "DESKHAVEN_* environment variable";
pub(crate) const ENV_FILE_LAYER: &str = "DESKHAVEN_*_FILE environment variable";
pub(crate) const CLI_LAYER: &str = "command-line flag";

/// Merges all layers: file < env < env `_FILE` < CLI. Defaults come from
/// serde, so a key absent from every layer has no metadata (= default).
pub(crate) fn build(file: Option<&Path>, env: &EnvPlan, file_values: Dict, cli: Dict) -> Figment {
    let mut figment = Figment::new();
    if let Some(path) = file {
        figment = figment.merge(NormalizedPins(Toml::file_exact(path)));
    }
    figment
        .merge(Layer::new(plain_env(env), env_metadata(ENV_LAYER, "")))
        .merge(Layer::new(
            Static(file_values),
            env_metadata(ENV_FILE_LAYER, "_FILE"),
        ))
        .merge(Layer::new(Static(cli), Metadata::named(CLI_LAYER)))
}

/// `Env::prefixed("DESKHAVEN_").split("__")` restricted to recognized keys;
/// everything else is reported as a warning instead of failing extraction.
fn plain_env(plan: &EnvPlan) -> Env {
    let known = plan.plain_paths();
    Env::prefixed(ENV_PREFIX)
        .split(KEY_SEPARATOR)
        .filter(move |key| known.contains(&key.as_str().to_ascii_lowercase()))
}

/// Error messages name the variable (`DESKHAVEN_LISTEN__ADMIN`), not a path.
fn env_metadata(name: &'static str, suffix: &'static str) -> Metadata {
    Metadata::named(name).interpolater(move |_: &Profile, keys: &[&str]| {
        let joined = keys.join(KEY_SEPARATOR).to_ascii_uppercase();
        format!("{ENV_PREFIX}{joined}{suffix}")
    })
}

/// A provider with an overridden identity.
struct Layer<P> {
    inner: P,
    metadata: Metadata,
}

impl<P> Layer<P> {
    fn new(inner: P, metadata: Metadata) -> Self {
        Self { inner, metadata }
    }
}

impl<P: Provider> Provider for Layer<P> {
    fn metadata(&self) -> Metadata {
        self.metadata.clone()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        self.inner.data()
    }
}

/// Precomputed values for the default profile.
struct Static(Dict);

impl Provider for Static {
    fn metadata(&self) -> Metadata {
        Metadata::named("static")
    }

    fn data(&self) -> Result<Map<Profile, Dict>, Error> {
        Ok(Profile::Default.collect(self.0.clone()))
    }
}
