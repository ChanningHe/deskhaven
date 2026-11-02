use std::fmt;
use std::path::PathBuf;

/// Fatal problems while loading the bootstrap configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file {} (from {origin}) does not exist or is not a regular file", path.display())]
    ConfigFileNotFound { path: PathBuf, origin: &'static str },

    #[error("both {var} and {file_var} are set; use only one")]
    EnvConflict { var: String, file_var: String },

    #[error("cannot read file {} referenced by {var}", path.display())]
    EnvFileRead {
        var: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("unknown configuration key `{key}` given on the command line")]
    UnknownCliKey { key: String },

    /// Parse/type/unknown-key errors reported by figment, one per line.
    #[error("invalid configuration:\n{0}")]
    Extract(String),

    /// Semantic validation failures (all collected, not just the first).
    #[error("invalid configuration:\n  - {}", problems.join("\n  - "))]
    Invalid { problems: Vec<String> },

    #[error("failed to render configuration")]
    Render(#[from] serde_json::Error),
}

impl From<figment::Error> for ConfigError {
    fn from(err: figment::Error) -> Self {
        let lines: Vec<String> = err.into_iter().map(|e| format!("  - {e}")).collect();
        Self::Extract(lines.join("\n"))
    }
}

/// Non-fatal findings returned to the caller; the library never prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A `DESKHAVEN_*` variable that maps to no configuration key.
    UnknownEnv { var: String },
    /// A `DESKHAVEN_*` variable whose name or value is not valid UTF-8.
    NonUnicodeEnv { var: String },
    /// `listen.admin` is split off but `listen.admin_public_url` is empty.
    AdminRedirectUnset,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEnv { var } => {
                write!(
                    f,
                    "environment variable {var} does not match any configuration key"
                )
            }
            Self::NonUnicodeEnv { var } => {
                write!(
                    f,
                    "environment variable {var} is not valid UTF-8 and was ignored"
                )
            }
            Self::AdminRedirectUnset => f.write_str(
                "listen.admin is set but listen.admin_public_url is empty; \
                 client_api \"/\" cannot redirect to the admin console",
            ),
        }
    }
}
