//! Bootstrap configuration: CLI > env > TOML file > built-in defaults.
//!
//! See `05-config.md` §2. Values needed before the database is reachable;
//! changing any of them requires a restart.

mod cli;
mod effective;
mod env;
mod error;
mod keys;
mod layers;
mod pins;
mod schema;
mod validate;

use std::path::{Path, PathBuf};

pub use cli::CliOverrides;
pub use effective::EffectiveEntry;
pub use error::{ConfigError, Warning};
pub use pins::{Pin, Pins};
pub use schema::{Bootstrap, Database, Listen, Log, LogFormat, Security, Signal, json_schema};

use crate::Source;
use env::CONFIG_VAR;
use keys::KnownKeys;
use pins::SETTINGS_KEY;

/// Config file candidates when neither `--config` nor `$DESKHAVEN_CONFIG` is set.
pub const DEFAULT_SEARCH_PATHS: [&str; 2] = ["deskhaven.toml", "/etc/deskhaven/deskhaven.toml"];

/// Inputs that do not come from the process environment.
#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// `--config <path>`; must exist when given.
    pub config_path: Option<PathBuf>,
    /// Tried in order when no explicit path is given; first existing file wins.
    pub search_paths: Vec<PathBuf>,
    pub cli: CliOverrides,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            config_path: None,
            search_paths: DEFAULT_SEARCH_PATHS.iter().map(PathBuf::from).collect(),
            cli: CliOverrides::default(),
        }
    }
}

/// A validated bootstrap configuration with provenance.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Bootstrap,
    /// The file that was read, if any.
    pub config_file: Option<PathBuf>,
    /// Non-fatal findings for the caller to log.
    pub warnings: Vec<Warning>,
    entries: Vec<EffectiveEntry>,
}

impl Loaded {
    /// Every key (schema leaves, then `settings.*` pins) with redacted values.
    pub fn effective(&self) -> &[EffectiveEntry] {
        &self.entries
    }

    /// Provenance of a dotted key; `None` if the key does not exist.
    pub fn source(&self, key: &str) -> Option<&Source> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| &e.source)
    }
}

/// Loads, merges and validates the bootstrap configuration. Reads the process
/// environment (`DESKHAVEN_*`) but never prints; warnings are returned.
pub fn load(options: &LoadOptions) -> Result<Loaded, ConfigError> {
    let known = KnownKeys::new()?;
    let env = env::plan(std::env::vars_os(), &known)?;
    let env_layers = env::layers(&env, &known)?;
    let cli = options.cli.to_dict(&known)?;
    let config_file = locate(options, env.config_path.as_deref())?;

    let figment = layers::build(config_file.as_deref(), env_layers, cli);
    let extracted: Bootstrap = figment
        .extract()
        .map_err(|e| ConfigError::extract(e, &known))?;
    let settings = extracted
        .settings
        .with_sources(|key| effective::source_of(&figment, &env, &format!("{SETTINGS_KEY}.{key}")));
    let config = Bootstrap {
        settings,
        ..extracted
    };

    let (problems, validation_warnings) = validate::validate(&config);
    if !problems.is_empty() {
        return Err(ConfigError::Invalid { problems });
    }
    let entries = effective::collect(&figment, &config, &env)?;
    let warnings = env
        .warnings
        .into_iter()
        .chain(validation_warnings)
        .collect();
    Ok(Loaded {
        config,
        config_file,
        warnings,
        entries,
    })
}

/// `--config` > `$DESKHAVEN_CONFIG` > first existing search path > none.
fn locate(options: &LoadOptions, env_path: Option<&Path>) -> Result<Option<PathBuf>, ConfigError> {
    let explicit = match (&options.config_path, env_path) {
        (Some(path), _) => Some((path.as_path(), "--config")),
        (None, Some(path)) => Some((path, CONFIG_VAR)),
        (None, None) => None,
    };
    if let Some((path, origin)) = explicit {
        return match path.is_file() {
            true => Ok(Some(absolute(path))),
            false => Err(ConfigError::ConfigFileNotFound {
                path: path.to_owned(),
                origin,
            }),
        };
    }
    Ok(options
        .search_paths
        .iter()
        .find(|p| p.is_file())
        .map(|p| absolute(p)))
}

/// Absolute form for clearer provenance; falls back to the path as given if
/// the working directory is unavailable (purely cosmetic).
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}
