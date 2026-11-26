//! Bootstrap configuration: CLI > env > TOML file > built-in defaults. Values needed before
//! the database is reachable; changing any of them requires a restart.

mod cli;
mod effective;
mod env;
mod error;
mod file;
mod keys;
mod layers;
mod pins;
mod schema;
mod schema_facts;
mod validate;

use std::path::{Path, PathBuf};

pub use cli::CliOverrides;
pub use effective::EffectiveEntry;
pub use env::env_var;
pub use error::{ConfigError, Warning};
pub use pins::{Pin, Pins};
pub use schema::{Bootstrap, Database, Listen, Log, LogFormat, Security, Signal, json_schema};

use crate::Source;
use env::CONFIG_VAR;
use keys::KnownKeys;
pub(crate) use pins::SETTINGS_KEY;

pub const DEFAULT_SEARCH_PATHS: [&str; 2] = ["deskhaven.toml", "/etc/deskhaven/deskhaven.toml"];

#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// Must exist when given.
    pub config_path: Option<PathBuf>,
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

#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Bootstrap,
    pub config_file: Option<PathBuf>,
    pub warnings: Vec<Warning>,
    entries: Vec<EffectiveEntry>,
}

impl Loaded {
    pub fn effective(&self) -> &[EffectiveEntry] {
        &self.entries
    }

    pub fn source(&self, key: &str) -> Option<&Source> {
        self.entries
            .iter()
            .find(|e| e.key == key)
            .map(|e| &e.source)
    }
}

/// Reads the process environment but never prints; warnings are returned.
pub fn load(options: &LoadOptions) -> Result<Loaded, ConfigError> {
    let known = KnownKeys::new()?;
    let env = env::plan(std::env::vars_os(), &known)?;
    let env_layers = env::layers(&env, &known)?;
    let cli = options.cli.to_dict(&known)?;
    let config_file = locate(options, env.config_path.as_deref())?;

    let file = match &config_file {
        Some(path) => Some((path.as_path(), file::read(path)?)),
        None => None,
    };
    let figment = layers::build(file, env_layers, cli);
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

/// Cosmetic: falls back to the path as given if the working directory is unavailable.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_owned())
}
