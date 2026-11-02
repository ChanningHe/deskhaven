//! Shared helpers for bootstrap integration tests.
//!
//! Every test runs inside `figment::Jail` (serialized, temp cwd, env restored
//! on exit) and calls `jail.clear_env()` first so the host environment never
//! leaks in.

#![allow(dead_code)]

use deskhaven_config::bootstrap::{CliOverrides, ConfigError, LoadOptions, Loaded};
use figment::Jail;

/// Options that only look for `deskhaven.toml` inside the jail directory, so a
/// host-wide `/etc/deskhaven/deskhaven.toml` never influences tests.
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

/// Converts a config error into a jail error so `?` works in jail closures.
pub fn jail_err(err: ConfigError) -> figment::Error {
    figment::Error::from(err.to_string())
}
