//! Provenance of an effective configuration value.

use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

/// Where an effective value came from.
///
/// Shared by bootstrap keys and runtime settings; only settings use
/// [`Source::Database`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[non_exhaustive]
pub enum Source {
    /// Built-in default; no layer set the key.
    Default,
    /// A TOML configuration file.
    File { path: PathBuf },
    /// An environment variable (including `<KEY>_FILE` indirection).
    Env { var: String },
    /// A command-line flag.
    Cli,
    /// A runtime setting stored in the database.
    Database {
        /// Optimistic-lock version of the stored row (starts at 1).
        version: i64,
        /// User who wrote it; `None` for the system.
        by: Option<i64>,
        /// When it was written, unix milliseconds (UTC).
        at: i64,
    },
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => f.write_str("default"),
            Self::File { path } => write!(f, "file:{}", path.display()),
            Self::Env { var } => write!(f, "env:{var}"),
            Self::Cli => f.write_str("cli"),
            Self::Database { version, .. } => write!(f, "database:v{version}"),
        }
    }
}
