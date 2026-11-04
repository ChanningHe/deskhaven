//! Provider and directory errors -> enveloped admin API errors (fixed English messages;
//! details name fields only).

use deskhaven_core::ldap::DirectoryError;
use deskhaven_core::oidc::ProviderError;
use serde_json::json;

use crate::domain_error::{invalid_field, refused};
use crate::{AdminApiError, ErrorCode};

impl From<ProviderError> for AdminApiError {
    fn from(err: ProviderError) -> Self {
        match err {
            ProviderError::NotFound => Self::not_found("OIDC provider not found"),
            ProviderError::Conflict => {
                Self::conflict("name is already taken").with_details(json!({ "field": "name" }))
            }
            ProviderError::Invalid { field, reason } => invalid_field(field, reason),
            ProviderError::ManagedByConfig => Self::new(
                ErrorCode::ManagedByConfig,
                "the OIDC provider is managed by the configuration file",
            ),
            ProviderError::InUse => Self::conflict(
                "accounts are linked to this provider: disable it instead, or unlink them first",
            )
            .with_details(json!({ "reason": "in_use" })),
            ProviderError::Refused(r) => refused(r),
            other => Self::internal(other),
        }
    }
}

impl From<DirectoryError> for AdminApiError {
    fn from(err: DirectoryError) -> Self {
        match err {
            DirectoryError::NotFound => Self::not_found("directory not found"),
            DirectoryError::Conflict { field: "enabled" } => {
                Self::conflict("another directory is already enabled")
                    .with_details(json!({ "field": "enabled" }))
            }
            DirectoryError::Conflict { field } => {
                Self::conflict(format!("{field} is already taken"))
                    .with_details(json!({ "field": field }))
            }
            DirectoryError::Invalid { field, reason } => invalid_field(field, reason),
            DirectoryError::ManagedByConfig => Self::new(
                ErrorCode::ManagedByConfig,
                "the directory is managed by the configuration file",
            ),
            DirectoryError::Refused(r) => refused(r),
            other => Self::internal(other),
        }
    }
}
