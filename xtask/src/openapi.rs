//! `xtask gen-openapi [--check]`: write the admin API's OpenAPI document to its committed
//! path (`deskhaven_admin_api::SPEC_PATH`), or with `--check` fail when the committed file
//! differs from what the code generates (CI drift check).

use std::path::Path;

use crate::error::XtaskError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Write,
    Check,
}

impl Mode {
    pub fn parse(args: &[String]) -> Result<Self, XtaskError> {
        match args {
            [] => Ok(Self::Write),
            [flag] if flag == "--check" => Ok(Self::Check),
            [other, ..] => Err(XtaskError::UnknownArgument(other.clone())),
        }
    }
}

pub fn run(root: &Path, mode: Mode) -> Result<(), XtaskError> {
    let spec = deskhaven_admin_api::openapi_json()?;
    let path = root.join(deskhaven_admin_api::SPEC_PATH);
    sync(&path, &spec, mode)?;
    eprintln!("xtask: {} is up to date", deskhaven_admin_api::SPEC_PATH);
    Ok(())
}

/// Makes `path` hold `spec` (write mode) or checks that it does (check mode).
fn sync(path: &Path, spec: &str, mode: Mode) -> Result<(), XtaskError> {
    crate::sync::file(path, spec, mode, XtaskError::OpenApiDrift)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_the_check_flag_only() {
        assert_eq!(Mode::parse(&args(&[])).unwrap(), Mode::Write);
        assert_eq!(Mode::parse(&args(&["--check"])).unwrap(), Mode::Check);
        for bad in [&["--fix"][..], &["--check", "x"]] {
            assert!(matches!(
                Mode::parse(&args(bad)),
                Err(XtaskError::UnknownArgument(_))
            ));
        }
    }

    #[test]
    fn check_detects_drift_and_a_missing_file_and_write_fixes_both() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("openapi.json");

        assert!(matches!(
            sync(&path, "{}\n", Mode::Check),
            Err(XtaskError::OpenApiDrift(_))
        ));
        sync(&path, "{}\n", Mode::Write).unwrap();
        sync(&path, "{}\n", Mode::Check).unwrap();
        assert!(matches!(
            sync(&path, "{\"x\":1}\n", Mode::Check),
            Err(XtaskError::OpenApiDrift(_))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}\n");
    }

    #[test]
    fn committed_spec_location_is_inside_the_repository() {
        let path = crate::repo_root().join(deskhaven_admin_api::SPEC_PATH);
        assert!(path.parent().unwrap().join("Cargo.toml").is_file());
    }
}
