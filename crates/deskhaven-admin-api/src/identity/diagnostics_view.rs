//! Wire types of the LDAP diagnostics: what to test (a saved directory or an unsaved form)
//! and the result of every step.

use deskhaven_core::ldap::{Diagnostic, ProbedUser, Step, StepReport, StepStatus};
use deskhaven_core::login_events::MAX_USERNAME_ATTEMPTED_CHARS;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::ldap_view::NewLdapDirectoryBody;

/// Longest accepted test password (characters), as for logins.
const MAX_PASSWORD_INPUT: usize = 1024;

/// "Test connection": connect and service bind. Give exactly one of `directory_id` (the
/// saved directory, with its stored password) and `directory` (an unsaved form, with the
/// password typed in it).
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LdapConnectionTest {
    #[garde(range(min = 1))]
    pub directory_id: Option<i64>,
    #[garde(dive)]
    pub directory: Option<NewLdapDirectoryBody>,
}

/// "Test user": every login step for `username`; the user bind only with a `password`,
/// which counts against that username's lockout like a login when it fails. Target as in
/// [`LdapConnectionTest`].
#[derive(Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LdapUserTest {
    #[garde(range(min = 1))]
    pub directory_id: Option<i64>,
    #[garde(dive)]
    pub directory: Option<NewLdapDirectoryBody>,
    #[garde(length(chars, min = 1, max = MAX_USERNAME_ATTEMPTED_CHARS))]
    pub username: String,
    /// Absent or empty: the user bind is skipped (nothing is counted).
    #[garde(length(chars, max = MAX_PASSWORD_INPUT))]
    #[schema(format = Password)]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LdapStep {
    Connect,
    ServiceBind,
    UserSearch,
    UserBind,
    GroupMapping,
}

impl From<Step> for LdapStep {
    fn from(step: Step) -> Self {
        match step {
            Step::Connect => Self::Connect,
            Step::ServiceBind => Self::ServiceBind,
            Step::UserSearch => Self::UserSearch,
            Step::UserBind => Self::UserBind,
            Step::GroupMapping => Self::GroupMapping,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LdapStepStatus {
    Passed,
    Failed,
    /// Not applicable, or not run after an earlier failure.
    Skipped,
}

impl From<StepStatus> for LdapStepStatus {
    fn from(status: StepStatus) -> Self {
        match status {
            StepStatus::Passed => Self::Passed,
            StepStatus::Failed => Self::Failed,
            StepStatus::Skipped => Self::Skipped,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct LdapStepView {
    pub step: LdapStep,
    pub status: LdapStepStatus,
    /// What happened (English, from the server and the directory; never a password).
    pub message: String,
}

impl From<StepReport> for LdapStepView {
    fn from(r: StepReport) -> Self {
        Self {
            step: r.step.into(),
            status: r.status.into(),
            message: r.message,
        }
    }
}

/// What the directory says about the tested user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct LdapProbedUserView {
    pub dn: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// Group DNs as found (empty when the group step did not run).
    pub directory_groups: Vec<String>,
    /// Console-managed deskhaven groups they map to.
    pub mapped_groups: Vec<String>,
    /// `null`: no admin group configured.
    pub admin: Option<bool>,
}

impl From<ProbedUser> for LdapProbedUserView {
    fn from(u: ProbedUser) -> Self {
        Self {
            dn: u.dn,
            username: u.username,
            email: u.email,
            display_name: u.display_name,
            directory_groups: u.directory_groups,
            mapped_groups: u.mapped_groups,
            admin: u.admin,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct LdapDiagnosticView {
    /// No step failed.
    pub passed: bool,
    /// In login order; a failed step skips the ones after it.
    pub steps: Vec<LdapStepView>,
    /// The tested user's entry, once found.
    pub user: Option<LdapProbedUserView>,
}

impl From<Diagnostic> for LdapDiagnosticView {
    fn from(d: Diagnostic) -> Self {
        Self {
            passed: d.passed(),
            steps: d.steps.into_iter().map(Into::into).collect(),
            user: d.user.map(Into::into),
        }
    }
}
