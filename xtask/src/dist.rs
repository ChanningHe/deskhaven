//! `xtask dist`: build the web console, then the release binary that embeds it.

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use crate::error::XtaskError;

/// One external command, run from the repository root.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub description: &'static str,
    pub program: OsString,
    pub args: Vec<&'static str>,
}

fn step(description: &'static str, program: &OsString, args: &[&'static str]) -> Step {
    Step {
        description,
        program: program.clone(),
        args: args.to_vec(),
    }
}

/// `cargo` is the program that started xtask (`$CARGO`) so the toolchain matches.
pub fn steps(cargo: &OsString) -> Vec<Step> {
    let pnpm = OsString::from("pnpm");
    vec![
        step(
            "install web dependencies",
            &pnpm,
            &["-C", "web", "install", "--frozen-lockfile"],
        ),
        step("build the web console", &pnpm, &["-C", "web", "build"]),
        // rust-embed cannot tell cargo that web/dist changed (or appeared), so force
        // the crate that embeds it to be rebuilt.
        step(
            "drop the stale embed",
            cargo,
            &["clean", "--release", "-p", "deskhaven-web"],
        ),
        step(
            "build the release binary",
            cargo,
            &["build", "--release", "-p", "deskhaven"],
        ),
    ]
}

pub fn run(root: &Path, cargo: &OsString) -> Result<(), XtaskError> {
    execute(root, &steps(cargo))?;
    eprintln!("xtask: done: target/release/deskhaven");
    Ok(())
}

fn execute(root: &Path, steps: &[Step]) -> Result<(), XtaskError> {
    for step in steps {
        eprintln!("xtask: {}", step.description);
        let status = Command::new(&step.program)
            .args(&step.args)
            .current_dir(root)
            .status()
            .map_err(|source| XtaskError::Spawn {
                program: step.program.to_string_lossy().into_owned(),
                source,
            })?;
        if !status.success() {
            return Err(XtaskError::Failed {
                step: step.description,
                status,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(program: &str) -> [Step; 1] {
        [step("probe", &OsString::from(program), &[])]
    }

    #[test]
    fn frontend_is_built_before_the_release_binary() {
        let lines: Vec<String> = steps(&OsString::from("cargo"))
            .iter()
            .map(|s| format!("{} {}", s.program.to_string_lossy(), s.args.join(" ")))
            .collect();
        assert_eq!(
            lines,
            [
                "pnpm -C web install --frozen-lockfile",
                "pnpm -C web build",
                "cargo clean --release -p deskhaven-web",
                "cargo build --release -p deskhaven",
            ]
        );
    }

    #[test]
    fn a_missing_program_is_reported_by_name() {
        let err = execute(Path::new("."), &probe("no-such-program-xyz")).unwrap_err();
        assert!(err.to_string().contains("no-such-program-xyz"), "{err}");
    }

    #[test]
    fn a_failing_step_stops_the_run_and_is_named() {
        let err = execute(Path::new("."), &probe("false")).unwrap_err();
        assert!(err.to_string().contains("probe"), "{err}");
    }
}
