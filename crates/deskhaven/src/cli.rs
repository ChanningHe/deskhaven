//! Command-line interface definition.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use deskhaven_config::bootstrap::{CliOverrides, LoadOptions};

#[derive(Debug, Parser)]
#[command(
    name = "deskhaven",
    version,
    about = "Self-hosted server for the RustDesk client"
)]
pub struct Cli {
    /// Configuration file [default: ./deskhaven.toml, then /etc/deskhaven/deskhaven.toml]
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Overrides `data_dir`
    #[arg(long, global = true, value_name = "PATH")]
    pub data_dir: Option<String>,

    /// Overrides `log.level` (tracing EnvFilter syntax)
    #[arg(long, global = true, value_name = "FILTER")]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the server (default)
    Serve,
    /// Inspect and validate configuration
    #[command(subcommand)]
    Config(ConfigCommand),
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Validate the configuration and exit
    Check,
    /// Print configuration values
    Print {
        /// Every key with its effective value (secrets redacted) and source
        #[arg(long, required = true)]
        effective: bool,
    },
    /// Print the JSON Schema of the bootstrap configuration file
    Schema,
}

impl Cli {
    /// Load options derived from global flags.
    pub fn load_options(&self) -> LoadOptions {
        let overrides = [("data_dir", &self.data_dir), ("log.level", &self.log_level)];
        let cli = overrides
            .into_iter()
            .fold(CliOverrides::default(), |acc, (key, value)| match value {
                Some(value) => acc.with(key, value.clone()),
                None => acc,
            });
        LoadOptions {
            config_path: self.config.clone(),
            cli,
            ..LoadOptions::default()
        }
    }
}
