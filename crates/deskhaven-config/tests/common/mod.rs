//! Every test runs inside `figment::Jail` and calls `jail.clear_env()` first so the host
//! environment never leaks in.

#![allow(dead_code)]

use deskhaven_config::bootstrap::{CliOverrides, ConfigError, LoadOptions, Loaded};
use figment::Jail;

/// Searches only the jail directory, so a host-wide config file never influences tests.
pub fn options(jail: &Jail) -> LoadOptions {
    LoadOptions {
        config_path: None,
        search_paths: vec![jail.directory().join("deskhaven.toml")],
        cli: CliOverrides::default(),
    }
}

pub fn load(jail: &Jail) -> Result<Loaded, ConfigError> {
    deskhaven_config::bootstrap::load(&options(jail))
}

pub fn jail_err(err: ConfigError) -> figment::Error {
    figment::Error::from(err.to_string())
}
