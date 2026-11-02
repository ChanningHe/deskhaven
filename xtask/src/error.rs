use std::io;
use std::process::ExitStatus;

#[derive(Debug, thiserror::Error)]
pub enum XtaskError {
    #[error("cannot run `{program}` (is it installed and on PATH? try `nix develop`)")]
    Spawn {
        program: String,
        #[source]
        source: io::Error,
    },
    #[error("step failed: {step} ({status})")]
    Failed {
        step: &'static str,
        status: ExitStatus,
    },
    #[error("unknown task `{0}`")]
    UnknownTask(String),
    #[error("no task given")]
    NoTask,
}
