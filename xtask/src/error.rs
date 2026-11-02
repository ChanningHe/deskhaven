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
    #[error("cannot generate the OpenAPI document")]
    OpenApi(#[from] deskhaven_admin_api::OpenApiError),
    #[error("{0} is out of date; run `cargo xtask gen-openapi` and commit the result")]
    OpenApiDrift(String),
    #[error("cannot access {path}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("unknown task `{0}`")]
    UnknownTask(String),
    #[error("unknown argument `{0}`")]
    UnknownArgument(String),
    #[error("no task given")]
    NoTask,
}
