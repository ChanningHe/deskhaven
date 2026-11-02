//! Write-side filesystem primitives for key files: 0700 directories and
//! atomic create-only writes (see the module docs of `keys`).

use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use super::KeyError;

/// Owner read/write: private keys.
#[cfg(unix)]
const PRIVATE_MODE: u32 = 0o600;
/// World readable: public keys (umask still applies), as upstream hbbs.
#[cfg(unix)]
const PUBLIC_MODE: u32 = 0o644;
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;
const TEMP_PREFIX: &str = ".deskhaven-key-";

#[derive(Debug, Clone, Copy)]
pub(super) enum Visibility {
    Private,
    Public,
}

/// Outcome of [`create_new`]; losing a race to another writer is not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Created {
    New,
    AlreadyExisted,
}

/// The directory holding `path` (`.` for a bare file name).
pub(super) fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Whether anything (including a dangling symlink) occupies `path`.
pub(super) fn occupied(path: &Path) -> Result<bool, KeyError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(KeyError::io("inspect", path, err)),
    }
}

/// Creates the directory holding `path` (and missing ancestors) as 0700.
/// Existing directories are left untouched.
pub(super) fn ensure_parent_dir(path: &Path) -> Result<(), KeyError> {
    let dir = parent_dir(path);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, DIR_MODE);
    builder
        .create(&dir)
        .map_err(|err| KeyError::io("create directory", &dir, err))
}

/// Writes `contents` to a temporary file beside `path`, syncs it, then
/// links it into place only if `path` is still free. A crash leaves either
/// no key or a complete one; an existing entry is never replaced.
pub(super) fn create_new(
    path: &Path,
    contents: &[u8],
    visibility: Visibility,
) -> Result<Created, KeyError> {
    let dir = parent_dir(path);
    let mut temp = temp_builder(visibility)
        .tempfile_in(&dir)
        .map_err(|err| KeyError::io("create a temporary file in", &dir, err))?;
    temp.write_all(contents)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|err| KeyError::io("write", temp.path(), err))?;
    match temp.persist_noclobber(path) {
        Ok(_) => sync_dir(&dir).map(|()| Created::New),
        Err(err) if err.error.kind() == ErrorKind::AlreadyExists => Ok(Created::AlreadyExisted),
        Err(err) => Err(KeyError::io("create", path, err.error)),
    }
}

fn temp_builder(visibility: Visibility) -> tempfile::Builder<'static, 'static> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(TEMP_PREFIX);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = match visibility {
            Visibility::Private => PRIVATE_MODE,
            Visibility::Public => PUBLIC_MODE,
        };
        builder.permissions(std::fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    let _ = visibility;
    builder
}

/// Makes the new directory entry durable.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<(), KeyError> {
    std::fs::File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|err| KeyError::io("sync directory", dir, err))
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<(), KeyError> {
    Ok(())
}
