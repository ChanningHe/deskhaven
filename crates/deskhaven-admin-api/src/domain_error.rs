//! Core service errors -> enveloped admin API errors. Messages are fixed English text;
//! `details` carries only field names and policy reasons, never input values.

use deskhaven_core::console_audit::Refused;
use deskhaven_core::devices::DeviceError;
use deskhaven_core::groups::GroupError;
use deskhaven_core::sessions::SessionError;
use deskhaven_core::tfa::TfaError;
use deskhaven_core::users::{PasswordError, UserError};
use serde_json::json;

use crate::{AdminApiError, ErrorCode};

/// [`Refused::code`] of the account rule "only an owner may manage owners" (06 §4.1).
pub(crate) const OWNER_REQUIRED: &str = "owner_required";

/// [`Refused::code`] of an actor that may not use the operation at all (no longer an
/// administrator). Like `ACTOR_INACTIVE` it maps to 403 `forbidden`.
pub(crate) const FORBIDDEN: &str = "forbidden";

/// One invalid field, in the `validation_failed` details shape of `ValidJson`.
pub(crate) fn invalid_field(field: &str, reason: impl ToString) -> AdminApiError {
    AdminApiError::validation_failed("invalid input")
        .with_details(json!({ "fields": { field: [reason.to_string()] } }))
}

pub(crate) fn refused(refused: Refused) -> AdminApiError {
    match refused.code {
        OWNER_REQUIRED => AdminApiError::new(
            ErrorCode::OwnerRequired,
            "only an owner may manage owner accounts",
        ),
        _ => AdminApiError::forbidden(),
    }
}

impl From<UserError> for AdminApiError {
    fn from(err: UserError) -> Self {
        match err {
            UserError::NotFound => Self::not_found("user not found"),
            UserError::Conflict { field } => Self::conflict(format!("{field} is already taken"))
                .with_details(json!({ "field": field })),
            UserError::Invalid { field, reason } => invalid_field(field, reason),
            UserError::PolicyViolation { reason } => invalid_field("password", reason),
            UserError::LastOwner => Self::new(
                ErrorCode::LastOwner,
                "the last active owner cannot be demoted, disabled or deleted",
            ),
            UserError::NotLocal => Self::new(
                ErrorCode::NotLocalAccount,
                "the account does not use a local password",
            ),
            UserError::Refused(r) => refused(r),
            UserError::Password(PasswordError::Busy) => {
                Self::new(ErrorCode::Unavailable, "server busy, try again later")
            }
            other => Self::internal(other),
        }
    }
}

impl From<GroupError> for AdminApiError {
    fn from(err: GroupError) -> Self {
        match err {
            GroupError::NotFound => Self::not_found("group not found"),
            GroupError::UserNotFound => Self::not_found("user not found"),
            GroupError::Conflict { field } => Self::conflict(format!("{field} is already taken"))
                .with_details(json!({ "field": field })),
            GroupError::Invalid { field, reason } => invalid_field(field, reason),
            GroupError::ManagedByConfig => Self::new(
                ErrorCode::ManagedByConfig,
                "the group is managed by the configuration file",
            ),
            GroupError::Refused(r) => refused(r),
            other => Self::internal(other),
        }
    }
}

impl From<DeviceError> for AdminApiError {
    fn from(err: DeviceError) -> Self {
        match err {
            DeviceError::NotFound => Self::not_found("device not found"),
            DeviceError::GroupNotFound => Self::not_found("device group not found"),
            DeviceError::Conflict { field } => Self::conflict(format!("{field} is already taken"))
                .with_details(json!({ "field": field })),
            DeviceError::Invalid { field, reason } => invalid_field(field, reason),
            DeviceError::ManagedByConfig => Self::new(
                ErrorCode::ManagedByConfig,
                "the device group is managed by the configuration file",
            ),
            DeviceError::Refused(r) => refused(r),
            DeviceError::QueueFull => Self::new(
                ErrorCode::Unavailable,
                "too many devices have pending disconnects",
            ),
            // `--deploy` / `--assign` (client API, API tokens); the console never trips these.
            DeviceError::UnknownDevice => Self::not_found("device not found"),
            DeviceError::NameNotFound { field, kind } => {
                invalid_field(field, format!("does not name an existing {kind}"))
            }
            // The id/uuid check belongs to the signal server; the console never trips it.
            err @ (DeviceError::UuidMismatch { .. }
            | DeviceError::Strategy(_)
            | DeviceError::AddressBook(_)
            | DeviceError::Aborted
            | DeviceError::Database(_)) => Self::internal(err),
        }
    }
}

impl From<TfaError> for AdminApiError {
    fn from(err: TfaError) -> Self {
        match err {
            TfaError::NotFound => Self::not_found("user not found"),
            TfaError::Refused(r) => refused(r),
            other => Self::internal(other),
        }
    }
}

impl From<SessionError> for AdminApiError {
    fn from(err: SessionError) -> Self {
        match err {
            SessionError::UserNotFound => Self::not_found("user not found"),
            SessionError::Refused(r) => refused(r),
            other => Self::internal(other),
        }
    }
}
