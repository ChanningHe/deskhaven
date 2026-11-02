//! Development tasks. Run as `cargo run -p xtask -- <task>`.
//!
//! - `dist`: build the web console and the release binary that embeds it.

mod dist;
mod error;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use error::XtaskError;

const USAGE: &str = "usage: cargo run -p xtask -- <task>\n\ntasks:\n  dist    build web/dist, then `cargo build --release -p deskhaven`";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match run(task.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: error: {err}");
            if matches!(err, XtaskError::UnknownTask(_) | XtaskError::NoTask) {
                eprintln!("{USAGE}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(task: Option<&str>) -> Result<(), XtaskError> {
    match task {
        Some("dist") => dist::run(repo_root(), &cargo()),
        Some(other) => Err(XtaskError::UnknownTask(other.to_owned())),
        None => Err(XtaskError::NoTask),
    }
}

/// xtask lives in `<root>/xtask`.
fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
}

fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_and_missing_tasks_are_errors() {
        assert!(matches!(run(Some("nope")), Err(XtaskError::UnknownTask(_))));
        assert!(matches!(run(None), Err(XtaskError::NoTask)));
    }

    #[test]
    fn repo_root_holds_the_workspace_manifest() {
        assert!(repo_root().join("Cargo.toml").is_file());
        assert!(repo_root().join("web").is_dir());
    }
}
