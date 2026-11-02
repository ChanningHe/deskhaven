//! Filesystem primitives for key files: optional reads, permission checks,
//! 0700 directories and atomic create-only writes.

use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

use zeroize::Zeroizing;

use super::{KeyError, KeyWarning};

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

fn io_error(op: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> KeyError {
    let path = path.to_path_buf();
    move |source| KeyError::Io { op, path, source }
}

/// The file's text, or `None` if it does not exist.
pub(super) fn read_optional(path: &Path) -> Result<Option<Zeroizing<String>>, KeyError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(Zeroizing::new(text))),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_error("read", path)(err)),
    }
}

pub(super) fn exists(path: &Path) -> Result<bool, KeyError> {
    path.try_exists().map_err(io_error("inspect", path))
}

/// Warns when group or others have any access to a private key file.
#[cfg(unix)]
pub(super) fn permission_warning(path: &Path) -> Result<Option<KeyWarning>, KeyError> {
    use std::os::unix::fs::PermissionsExt;
    const PERMISSION_BITS: u32 = 0o7777;
    const GROUP_OTHER_BITS: u32 = 0o077;

    let mode = std::fs::metadata(path)
        .map_err(io_error("inspect", path))?
        .permissions()
        .mode()
        & PERMISSION_BITS;
    Ok(
        (mode & GROUP_OTHER_BITS != 0).then(|| KeyWarning::PermissiveMode {
            path: path.to_path_buf(),
            mode,
        }),
    )
}

#[cfg(not(unix))]
pub(super) fn permission_warning(_path: &Path) -> Result<Option<KeyWarning>, KeyError> {
    Ok(None)
}

fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
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
        .map_err(io_error("create directory", &dir))
}

/// Writes `contents` to a temporary file beside `path`, syncs it, then
/// links it into place only if `path` does not exist yet. A crash leaves
/// either no key or a complete one; an existing key is never replaced.
pub(super) fn create_new(
    path: &Path,
    contents: &[u8],
    visibility: Visibility,
) -> Result<(), KeyError> {
    let dir = parent_dir(path);
    let mut temp = temp_builder(visibility)
        .tempfile_in(&dir)
        .map_err(io_error("create a temporary file in", &dir))?;
    temp.write_all(contents)
        .and_then(|()| temp.as_file().sync_all())
        .map_err(io_error("write", temp.path()))?;
    temp.persist_noclobber(path)
        .map_err(|err| io_error("create", path)(err.error))?;
    sync_dir(&dir)
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
        .map_err(io_error("sync directory", dir))
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<(), KeyError> {
    Ok(())
}
