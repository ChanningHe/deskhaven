//! Generated files: write them, or check that the committed copy is current.

use std::fs;
use std::io;
use std::path::Path;

use crate::error::XtaskError;
use crate::openapi::Mode;

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
