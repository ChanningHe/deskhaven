use sea_orm::DbErr;

use super::Target;
use crate::console_audit::{GuardError, Refused};

/// Why a strategy operation failed. Variants are stable for API error mapping; database
/// details stay in the source chain.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StrategyError {
    #[error("strategy not found")]
    NotFound,

    /// The assignment target (user, group, device group or device) does not exist.
    #[error("assignment target not found: {0:?}")]
    TargetNotFound(Target),

    #[error("{field} is already taken")]
    Conflict { field: &'static str },

    /// Malformed input; `field` names the offending input field.
    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    /// The strategy is defined in the configuration file and read-only here.
    #[error("the strategy is managed by the configuration file")]
    ManagedByConfig,

    /// The console guard of a write declined it (nothing was changed).
    #[error(transparent)]
    Refused(#[from] Refused),

    /// The operation's task panicked or the runtime is shutting down; whether it committed
    /// is unknown until the next read.
    #[error("strategy operation did not complete")]
    Aborted,

    #[error("strategy storage failed")]
    Database(#[source] DbErr),
}

impl From<DbErr> for StrategyError {
    fn from(err: DbErr) -> Self {
        Self::Database(err)
    }
}

impl From<GuardError> for StrategyError {
    fn from(err: GuardError) -> Self {
        match err {
            GuardError::Refused(refused) => Self::Refused(refused),
            GuardError::Database(db) => Self::Database(db),
        }
    }
}
