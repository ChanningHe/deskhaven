//! `xtask loadgen [args]`: build the server and the load generator in release mode, then run
//! `deskhaven-loadgen --server-bin target/release/deskhaven [args]` (M4-04). The generator
//! is its own crate so xtask keeps its few dependencies and the simulated clients run
//! optimized; `deskhaven-loadgen --help` lists the arguments.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::XtaskError;

const BUILD_ARGS: [&str; 6] = [
    "build",
    "--release",
    "-p",
    "deskhaven",
    "-p",
    "deskhaven-loadgen",
];
/// Read by the generator to print how the run was started.
const INVOCATION_ENV: &str = "DESKHAVEN_LOADGEN_INVOCATION";

/// The generator binary and its arguments.
pub fn loadgen_command(target_dir: &Path, args: &[String]) -> (PathBuf, Vec<OsString>) {
    let release = target_dir.join("release");
    let server = release.join("deskhaven");
    let argv = [OsString::from("--server-bin"), server.into_os_string()]
        .into_iter()
        .chain(args.iter().map(OsString::from))
        .collect();
    (release.join("deskhaven-loadgen"), argv)
}

pub fn run(root: &Path, cargo: &OsString, args: &[String]) -> Result<(), XtaskError> {
    eprintln!("xtask: build the release server and load generator");
    let status = Command::new(cargo)
        .args(BUILD_ARGS)
        .current_dir(root)
        .status()
        .map_err(|source| XtaskError::Spawn {
            program: cargo.to_string_lossy().into_owned(),
            source,
        })?;
    if !status.success() {
        return Err(XtaskError::Failed {
            step: "build the release server and load generator",
            status,
        });
    }
    let (program, argv) = loadgen_command(&target_dir(root), args);
    let status = Command::new(&program)
        .args(&argv)
        .env(
            INVOCATION_ENV,
            format!("cargo xtask loadgen {}", args.join(" ")),
        )
        .current_dir(root)
        .status()
        .map_err(|source| XtaskError::Spawn {
            program: program.display().to_string(),
            source,
        })?;
    match status.success() {
        true => Ok(()),
        false => Err(XtaskError::Failed {
            step: "load test (status 2: an NFR failed, 1: error)",
            status,
        }),
    }
}

/// `$CARGO_TARGET_DIR` (relative to the root) or `<root>/target`.
fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), |dir| root.join(dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loadgen_runs_against_the_release_server_with_the_given_args() {
        let args = vec!["--devices".to_owned(), "10".to_owned()];

        let (program, argv) = loadgen_command(Path::new("/r/target"), &args);

        assert_eq!(program, Path::new("/r/target/release/deskhaven-loadgen"));
        assert_eq!(
            argv,
            [
                "--server-bin",
                "/r/target/release/deskhaven",
                "--devices",
                "10"
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn both_binaries_are_built_in_release_mode() {
        assert!(BUILD_ARGS.contains(&"--release"));
        assert!(BUILD_ARGS.contains(&"deskhaven") && BUILD_ARGS.contains(&"deskhaven-loadgen"));
    }
}
