//! Account setup a console session must finish before anything else (06 §3.3; M3-02):
//!
//! 1. `password_change`: a local account with `must_change_password` (a temporary password
//!    set by an administrator);
//! 2. `totp_enrolment`: `auth.tfa.enforce` covers the role, the account has no TOTP, and
//!    it signs in with a local factor (OIDC accounts are left to their IdP's MFA policy,
//!    as at OIDC login).
//!
//! The gate lets such a session reach only [`Permission::Session`] routes (`/me`, the
//! password change, the TOTP enrolment) and `Public` ones (logout); everything else is
//! 403 `setup_required` with `details.step`. A route guard in the SPA is a convenience on
//! top, not the control. Finishing a step mints a fresh session (password change: every
//! session was revoked; TOTP: the console sessions are rotated), so the restricted token
//! never gains the new standing.
//!
//! [`Permission::Session`]: crate::Permission::Session

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

/// The first unfinished step for an account of `role` with `flags`, or `None`.
pub fn pending_step(tfa: &TfaService, role: Role, flags: AccountFlags) -> Option<SetupStep> {
    if flags.auth_source == AuthSource::Local && flags.must_change_password {
        return Some(SetupStep::PasswordChange);
    }
    let local_factor = flags.auth_source != AuthSource::Oidc;
    if local_factor && !flags.totp_enabled && tfa.enforced_for(role) {
        return Some(SetupStep::TotpEnrolment);
    }
    None
}

/// 403 `setup_required` naming `step`.
pub(crate) fn setup_required(step: SetupStep) -> AdminApiError {
    AdminApiError::new(ErrorCode::SetupRequired, "finish the account setup first")
        .with_details(json!({ "step": step.as_str() }))
}
