//! `xtask gen-openapi [--check]`: write the admin API's OpenAPI document to its committed
//! path (`deskhaven_admin_api::SPEC_PATH`), or with `--check` fail when the committed file
//! differs from what the code generates (CI drift check).

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

#[cfg(test)]
mod tests {
    #[test]
    fn committed_spec_location_is_inside_the_repository() {
        let path = crate::repo_root().join(deskhaven_admin_api::SPEC_PATH);
        assert!(path.parent().unwrap().join("Cargo.toml").is_file());
    }
}
