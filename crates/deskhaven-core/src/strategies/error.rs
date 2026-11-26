use sea_orm::DbErr;

use super::Target;
use crate::console_audit::{GuardError, Refused};
use crate::crypto::CryptoError;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StrategyError {
    #[error("strategy not found")]
    NotFound,

    #[error("assignment target not found: {0:?}")]
    TargetNotFound(Target),

    #[error("{field} is already taken")]
    Conflict { field: &'static str },

    #[error("invalid {field}: {reason}")]
    Invalid { field: &'static str, reason: String },

    #[error("the strategy is managed by the configuration file")]
    ManagedByConfig,

    #[error(transparent)]
    Refused(#[from] Refused),

    /// Whether it committed is unknown until the next read.
    #[error("strategy operation did not complete")]
    Aborted,

    #[error("strategy storage failed")]
    Database(#[source] DbErr),

    #[error("sealing a secret strategy option failed")]
    Crypto(#[source] CryptoError),
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
