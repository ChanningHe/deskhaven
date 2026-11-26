//! Until setup is finished a console session reaches only `Session` and `Public` routes; the
//! SPA route guard is a convenience, not the control. Finishing a step mints a fresh
//! session, so the restricted token never gains the new standing.

use deskhaven_core::sessions::AccountFlags;
use deskhaven_core::tfa::TfaService;
use deskhaven_core::users::{AuthSource, Role};
use serde::Serialize;
use serde_json::json;
use utoipa::ToSchema;

use crate::{AdminApiError, ErrorCode};

/// The next account setup step of a console session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SetupStep {
    PasswordChange,
    TotpEnrolment,
}

impl SetupStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PasswordChange => "password_change",
            Self::TotpEnrolment => "totp_enrolment",
        }
    }
}

pub fn pending_step(tfa: &TfaService, role: Role, flags: AccountFlags) -> Option<SetupStep> {
    if flags.auth_source == AuthSource::Local && flags.must_change_password {
        return Some(SetupStep::PasswordChange);
    }
    // Password sign-ins (local or directory) carry the factor; OIDC defers to its IdP.
    let local_factor = flags.auth_source != AuthSource::Oidc;
    if local_factor && !flags.totp_enabled && tfa.enforced_for(role) {
        return Some(SetupStep::TotpEnrolment);
    }
    None
}

pub(crate) fn setup_required(step: SetupStep) -> AdminApiError {
    AdminApiError::new(ErrorCode::SetupRequired, "finish the account setup first")
        .with_details(json!({ "step": step.as_str() }))
}
