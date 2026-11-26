use std::path::Path;

use crate::error::XtaskError;
use crate::sync::{self, Mode};

pub fn run(root: &Path, mode: Mode) -> Result<(), XtaskError> {
    let spec = deskhaven_admin_api::openapi_json()?;
    let path = root.join(deskhaven_admin_api::SPEC_PATH);
    sync::file(&path, &spec, mode, XtaskError::OpenApiDrift)?;
    eprintln!("xtask: {} is up to date", deskhaven_admin_api::SPEC_PATH);
    Ok(())
}
