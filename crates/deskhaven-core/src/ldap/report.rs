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
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbedUser {
    pub dn: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub directory_groups: Vec<String>,
    pub mapped_groups: Vec<String>,
    pub admin: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostic {
    pub steps: Vec<StepReport>,
    pub user: Option<ProbedUser>,
    /// The password was sent to the directory, or no single entry matched (a login binds a decoy
    /// then).
    pub attempted: bool,
}

impl Diagnostic {
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
