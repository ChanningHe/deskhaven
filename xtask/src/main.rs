//! Development tasks. Run as `cargo xtask <task>` (alias of `cargo run -p xtask --`).
//!
//! - `dist`: build the web console and the release binary that embeds it.
//! - `gen-openapi [--check]`: write (or check) the admin API OpenAPI document.
//! - `gen-strategy-catalog [--from <dir>] [--check]`: regenerate (or check) the strategy
//!   editor's client option catalog from the vendored client source (or `<dir>`).

mod catalog;
mod dist;
mod error;
mod openapi;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use error::XtaskError;

const USAGE: &str = "usage: cargo xtask <task> [args]\n\ntasks:\n  dist                   build web/dist, then `cargo build --release -p deskhaven`\n  gen-openapi [--check]  write the admin API OpenAPI document (--check: fail on drift)\n  gen-strategy-catalog [--from <dir>] [--check]\n                         regenerate the strategy option catalog from the client source";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let task = args.next();
    let rest: Vec<String> = args.collect();
    match run(task.as_deref(), &rest) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask: error: {err}");
            let mut source = std::error::Error::source(&err);
            while let Some(cause) = source {
                eprintln!("  caused by: {cause}");
                source = cause.source();
            }
            if matches!(err, XtaskError::UnknownTask(_) | XtaskError::NoTask) {
                eprintln!("{USAGE}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run(task: Option<&str>, args: &[String]) -> Result<(), XtaskError> {
    match task {
        Some("dist") => dist::run(repo_root(), &cargo()),
        Some("gen-openapi") => openapi::run(repo_root(), openapi::Mode::parse(args)?),
        Some("gen-strategy-catalog") => catalog::run(repo_root(), &catalog::Args::parse(args)?),
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
        assert!(matches!(
            run(Some("nope"), &[]),
            Err(XtaskError::UnknownTask(_))
        ));
        assert!(matches!(run(None, &[]), Err(XtaskError::NoTask)));
    }

    #[test]
    fn repo_root_holds_the_workspace_manifest() {
        assert!(repo_root().join("Cargo.toml").is_file());
        assert!(repo_root().join("web").is_dir());
    }
}
