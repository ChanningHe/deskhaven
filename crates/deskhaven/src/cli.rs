//! Command-line interface definition.

use std::path::PathBuf;

use anyhow::Context;
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
    pub data_dir: Option<PathBuf>,

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
    /// Inspect key material
    #[command(subcommand)]
    Key(KeyCommand),
}

#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    /// Print the public key (the value clients enter as "Key")
    Show,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Validate the configuration and exit
    Check,
    /// Print explicitly set keys (secrets redacted) with their source
    Print {
        /// Include keys left at their built-in default
        #[arg(long)]
        effective: bool,
    },
    /// Print the JSON Schema of the bootstrap configuration file
    Schema,
}

impl Cli {
    /// Load options derived from global flags.
    pub fn load_options(&self) -> anyhow::Result<LoadOptions> {
        let mut cli = CliOverrides::default();
        if let Some(dir) = &self.data_dir {
            let dir = dir.to_str().context("--data-dir must be valid UTF-8")?;
            cli = cli.with("data_dir", dir);
        }
        if let Some(level) = &self.log_level {
            cli = cli.with("log.level", level.clone());
        }
        Ok(LoadOptions {
            config_path: self.config.clone(),
            cli,
            ..LoadOptions::default()
        })
    }
}
