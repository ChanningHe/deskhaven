//! Generated files: write them, or check that the committed copy is current.

use std::fs;
use std::io;
use std::path::Path;

use crate::error::XtaskError;

/// Write the generated file, or (`--check`) only compare it.
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

/// Makes `path` hold `content` (write mode) or checks that it does (check mode). A missing
/// file counts as drift; `drift` builds the error from the path.
pub fn file(
    path: &Path,
    content: &str,
    mode: Mode,
    drift: impl FnOnce(String) -> XtaskError,
) -> Result<(), XtaskError> {
    let current = match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(source) => return Err(io_error(path, source)),
    };
    if current.as_deref() == Some(content) {
        return Ok(());
    }
    match mode {
        Mode::Check => Err(drift(path.display().to_string())),
        Mode::Write => fs::write(path, content).map_err(|source| io_error(path, source)),
    }
}

fn io_error(path: &Path, source: io::Error) -> XtaskError {
    XtaskError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[cfg(test)]
mod tests {
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
        let path = dir.path().join("generated.json");
        let check = |content| file(&path, content, Mode::Check, XtaskError::OpenApiDrift);

        assert!(matches!(check("{}\n"), Err(XtaskError::OpenApiDrift(_))));
        file(&path, "{}\n", Mode::Write, XtaskError::OpenApiDrift).unwrap();
        check("{}\n").unwrap();
        assert!(matches!(
            check("{\"x\":1}\n"),
            Err(XtaskError::OpenApiDrift(_))
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{}\n");
    }
}
