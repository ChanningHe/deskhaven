use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

/// Only runtime settings use [`Source::Database`].
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
        /// Starts at 1.
        version: i64,
        /// User who wrote it; `None` for the system.
        by: Option<i64>,
        /// Unix milliseconds (UTC).
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
