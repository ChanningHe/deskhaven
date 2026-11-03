use deskhaven_config::settings::{Problem, Rejection};
use sea_orm::DbErr;

use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;

/// Why a settings update was not applied. Nothing is ever partially written.
/// Messages carry keys and reasons, never values.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SettingsError {
    /// Keys fixed by the bootstrap configuration (API: 409 `setting_pinned`).
    #[error("settings pinned by the bootstrap configuration: {}", keys.join(", "))]
    Pinned { keys: Vec<String> },

    /// Keys changed since the caller read them (API: 409).
    #[error("settings changed since they were read: {}", keys.join(", "))]
    VersionConflict { keys: Vec<String> },

    /// Unknown keys, invalid values or violated cross-key rules (API: 422).
    #[error("invalid settings: {}", render(problems))]
    Invalid { problems: Vec<Problem> },

    /// The audit guard of the write declined it (the acting account may no longer change
    /// settings); nothing was written.
    #[error(transparent)]
    Refused(#[from] Refused),

    /// The update task panicked or the runtime is shutting down; whether
    /// the batch was committed is unknown until the next read.
    #[error("settings update did not complete")]
    Aborted,

    /// The batch did not reach commit in time (e.g. another writer held the
    /// lock); nothing was written.
    #[error("settings update timed out")]
    Timeout,

    #[error("settings storage failed")]
    Database(#[from] DbErr),

    #[error("failed to encrypt a sensitive setting")]
    Crypto(#[from] CryptoError),
}

impl From<Rejection> for SettingsError {
    fn from(rejection: Rejection) -> Self {
        match rejection {
            Rejection::Pinned { keys } => Self::Pinned { keys },
            Rejection::Invalid { problems } => Self::Invalid { problems },
        }
    }
}

impl From<GuardError> for SettingsError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}

fn render(problems: &[Problem]) -> String {
    let rendered: Vec<String> = problems.iter().map(ToString::to_string).collect();
    rendered.join("; ")
}
