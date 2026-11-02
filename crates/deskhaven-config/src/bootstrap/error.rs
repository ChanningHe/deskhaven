use std::fmt;
use std::path::PathBuf;

use figment::error::{Actual, Kind};

use super::keys::KnownKeys;
use crate::types::REDACTED;

/// Fatal problems while loading the bootstrap configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file {} (from {origin}) does not exist or is not a regular file", path.display())]
    ConfigFileNotFound { path: PathBuf, origin: &'static str },

    #[error("cannot read config file {}", path.display())]
    ConfigFileRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Position and short reason only; the source line is never quoted.
    #[error("TOML syntax error at {line}:{column} in {}: {reason}", path.display())]
    TomlSyntax {
        path: PathBuf,
        line: usize,
        column: usize,
        reason: String,
    },

    #[error("both {var} and {file_var} are set; use only one")]
    EnvConflict { var: String, file_var: String },

    /// Two variables whose names differ only by case (names sorted).
    #[error("environment variables {first} and {second} set the same key; keep only one")]
    EnvDuplicate { first: String, second: String },

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

impl ConfigError {
    /// Renders figment errors without any found value, for every key: only
    /// key path, expected type, the found value's shape and the source remain.
    /// Secret keys additionally lose free-form messages.
    pub(crate) fn extract(err: figment::Error, known: &KnownKeys) -> Self {
        let lines: Vec<String> = err
            .into_iter()
            .map(|mut e| {
                let is_secret = known.is_secret(&e.path.join("."));
                e.kind = without_value(e.kind, is_secret);
                format!("  - {e}")
            })
            .collect();
        Self::Extract(lines.join("\n"))
    }
}

fn without_value(kind: Kind, is_secret: bool) -> Kind {
    let message = match kind {
        Kind::InvalidType(found, expected) => {
            format!(
                "invalid type: expected {expected} (found {})",
                shape(&found)
            )
        }
        Kind::InvalidValue(found, expected) => {
            format!(
                "invalid value: expected {expected} (found {})",
                shape(&found)
            )
        }
        Kind::UnknownVariant(_, expected) => {
            format!(
                "unknown variant: expected one of `{}`",
                expected.join("`, `")
            )
        }
        Kind::ISizeOutOfRange(_) | Kind::USizeOutOfRange(_) => "number out of range".to_owned(),
        Kind::Unsupported(found) => format!("unsupported value (found {})", shape(&found)),
        Kind::UnsupportedKey(found, expected) => {
            format!(
                "unsupported key: expected {expected} (found {})",
                shape(&found)
            )
        }
        Kind::Message(_) if is_secret => format!("invalid value ({REDACTED})"),
        // Field names and lengths carry no values; our own types' messages
        // never include their input.
        other => return other,
    };
    Kind::Message(message)
}

/// The kind of a found value, never its content.
fn shape(found: &Actual) -> &'static str {
    match found {
        Actual::Bool(_) => "a boolean",
        Actual::Unsigned(_) | Actual::Signed(_) => "an integer",
        Actual::Float(_) => "a float",
        Actual::Char(_) => "a character",
        Actual::Str(_) => "a string",
        Actual::Bytes(_) => "bytes",
        Actual::Seq => "a list",
        Actual::Map => "a table",
        Actual::Unit | Actual::Option => "an empty value",
        _ => "a value",
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
