//! The result of a diagnostic (`diagnose`): one report per step, the user found, and
//! whether a login would have counted the attempt.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Connect,
    ServiceBind,
    UserSearch,
    UserBind,
    GroupMapping,
}

impl Step {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::ServiceBind => "service_bind",
            Self::UserSearch => "user_search",
            Self::UserBind => "user_bind",
            Self::GroupMapping => "group_mapping",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    Passed,
    Failed,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepReport {
    pub step: Step,
    pub status: StepStatus,
    /// What happened, for the admin (English).
    pub message: String,
}

/// What the directory says about a tested user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbedUser {
    pub dn: String,
    /// The username attribute's first value.
    pub username: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    /// Group DNs as found (empty when no group step ran).
    pub directory_groups: Vec<String>,
    /// Console-managed deskhaven groups they map to.
    pub mapped_groups: Vec<String>,
    /// `None`: no admin group configured.
    pub admin: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostic {
    pub steps: Vec<StepReport>,
    /// The tested user's entry, once found.
    pub user: Option<ProbedUser>,
    /// A login with this username and password would have counted an attempt: the
    /// password was sent to the directory (whatever happened next), or the directory
    /// answered that no single entry matches (a login binds a decoy then). `false` when the
    /// directory failed before that, or no password was given.
    pub attempted: bool,
}

impl Diagnostic {
    /// No step failed.
    pub fn passed(&self) -> bool {
        self.steps.iter().all(|s| s.status != StepStatus::Failed)
    }
}

pub(super) const CONNECTION_STEPS: &[Step] = &[Step::Connect, Step::ServiceBind];
pub(super) const USER_STEPS: &[Step] = &[
    Step::Connect,
    Step::ServiceBind,
    Step::UserSearch,
    Step::UserBind,
    Step::GroupMapping,
];

/// Collects step results; unreported planned steps end up skipped.
pub(super) struct Recorder {
    plan: &'static [Step],
    pub(super) diagnostic: Diagnostic,
}

impl Recorder {
    pub(super) fn new(plan: &'static [Step]) -> Self {
        Self {
            plan,
            diagnostic: Diagnostic::default(),
        }
    }

    pub(super) fn record(&mut self, step: Step, status: StepStatus, message: impl Into<String>) {
        self.diagnostic.steps.push(StepReport {
            step,
            status,
            message: message.into(),
        });
    }

    /// The total time ran out: every planned step not reported yet failed.
    pub(super) fn time_out(&mut self) {
        self.fail_remaining("timed out: the test took too long and was stopped");
    }

    fn fail_remaining(&mut self, message: &str) {
        for step in self.plan {
            if !self.diagnostic.steps.iter().any(|s| s.step == *step) {
                self.record(*step, StepStatus::Failed, message);
            }
        }
    }

    pub(super) fn finish(mut self) -> Diagnostic {
        for step in self.plan {
            if !self.diagnostic.steps.iter().any(|s| s.step == *step) {
                self.record(
                    *step,
                    StepStatus::Skipped,
                    "skipped after an earlier failure",
                );
            }
        }
        self.diagnostic
    }
}
