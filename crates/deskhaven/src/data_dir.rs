//! `data_dir`: keys, the SQLite database and other private files.

use std::fmt;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Owner-only, for directories this process creates.
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;
/// Group/other permission bits.
#[cfg(unix)]
const GROUP_OTHER_BITS: u32 = 0o077;
#[cfg(unix)]
const PERMISSION_BITS: u32 = 0o7777;

/// An existing data_dir that group or others can access. Not fatal: the
/// operator may have reasons (shared volumes), but should know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LooseMode {
    pub path: PathBuf,
    pub mode: u32,
}

impl fmt::Display for LooseMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "data_dir {} has mode {:04o}; it holds keys and the database, restrict it (chmod 700 {0})",
            self.path.display(),
            self.mode
        )
    }
}

/// Creates `dir` (and missing parents) as 0700, or checks an existing one.
/// Existing directories are never chmod-ed.
pub fn prepare(dir: &Path) -> anyhow::Result<Option<LooseMode>> {
    match std::fs::metadata(dir) {
        Ok(meta) if meta.is_dir() => Ok(loose_mode(dir, &meta)),
        Ok(_) => bail!("data_dir {} is not a directory", dir.display()),
        Err(err) if err.kind() == ErrorKind::NotFound => {
            create(dir)?;
            tracing::info!(path = %dir.display(), "created data_dir");
            Ok(None)
        }
        Err(err) => Err(err).with_context(|| format!("cannot inspect data_dir {}", dir.display())),
    }
}

fn create(dir: &Path) -> anyhow::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, DIR_MODE);
    builder
        .create(dir)
        .with_context(|| format!("cannot create data_dir {}", dir.display()))
}

#[cfg(unix)]
fn loose_mode(dir: &Path, meta: &std::fs::Metadata) -> Option<LooseMode> {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode() & PERMISSION_BITS;
    (mode & GROUP_OTHER_BITS != 0).then(|| LooseMode {
        path: dir.to_path_buf(),
        mode,
    })
}

#[cfg(not(unix))]
fn loose_mode(_dir: &Path, _meta: &std::fs::Metadata) -> Option<LooseMode> {
    None
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used)] // helpers outside #[test] fns
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS
    }

    #[test]
    fn creates_missing_directory_owner_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b");

        let warning = prepare(&dir).unwrap();

        assert_eq!(warning, None);
        assert_eq!(mode_of(&dir), DIR_MODE);
    }

    #[test]
    fn warns_on_group_or_other_access_without_chmod() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

        let warning = prepare(tmp.path()).unwrap();

        assert_eq!(warning.map(|w| w.mode), Some(0o755));
        assert_eq!(mode_of(tmp.path()), 0o755);
    }

    #[test]
    fn accepts_owner_only_directory() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(prepare(tmp.path()).unwrap(), None);
    }

    #[test]
    fn rejects_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, b"").unwrap();

        let err = prepare(&file).unwrap_err();

        assert!(err.to_string().contains("not a directory"), "{err}");
    }
}
