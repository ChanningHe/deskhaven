//! Strategy service errors -> enveloped admin API errors. Messages are fixed English text;
//! `details` carries only field names and policy reasons, never input values.

use deskhaven_core::strategies::StrategyError;
use serde_json::json;

use crate::domain_error::{invalid_field, refused};
use crate::{AdminApiError, ErrorCode};

impl From<StrategyError> for AdminApiError {
    fn from(err: StrategyError) -> Self {
        match err {
            StrategyError::NotFound => Self::not_found("strategy not found"),
            // Only reachable from a request body (`target_id`): an input error.
            StrategyError::TargetNotFound(_) => {
                invalid_field("target_id", "does not name an existing target")
            }
            StrategyError::Conflict { field } => {
                Self::conflict(format!("{field} is already taken"))
                    .with_details(json!({ "field": field }))
            }
            StrategyError::Invalid { field, reason } => invalid_field(field, reason),
            StrategyError::ManagedByConfig => Self::new(
                ErrorCode::ManagedByConfig,
                "the strategy is managed by the configuration file",
            ),
            StrategyError::Refused(r) => refused(r),
            other => Self::internal(other),
        }
    }
}
