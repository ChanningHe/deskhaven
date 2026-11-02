//! Hardened key-file reads: no symlinks, regular files only, size-capped,
//! plus permission/ownership/directory warnings for private keys.

use std::fs::File;
use std::io::{ErrorKind, Read as _};
use std::path::Path;

use zeroize::Zeroizing;

use super::{KeyError, KeyWarning};

/// Key files are a few dozen bytes; anything larger is refused unread.
pub const MAX_KEY_FILE_LEN: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Sensitivity {
    Private,
    Public,
}

pub(super) struct KeyFile {
    pub(super) text: Zeroizing<String>,
    pub(super) warnings: Vec<KeyWarning>,
}

/// The file's text, or `None` if nothing exists at `path`.
pub(super) fn read_key_file(
    path: &Path,
    sensitivity: Sensitivity,
) -> Result<Option<KeyFile>, KeyError> {
    match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(KeyError::io("inspect", path, err)),
        Ok(meta) if meta.file_type().is_symlink() => return Err(symlink(path)),
        Ok(meta) if !meta.is_file() => return Err(not_regular(path)),
        Ok(_) => {}
    }
    let Some(file) = open_no_follow(path)? else {
        return Ok(None);
    };
    let meta = file
        .metadata()
        .map_err(|err| KeyError::io("inspect", path, err))?;
    if !meta.is_file() {
        return Err(not_regular(path));
    }
    if meta.len() > MAX_KEY_FILE_LEN as u64 {
        return Err(too_large(path));
    }
    let text = read_capped(file, path)?;
    let warnings = match sensitivity {
        Sensitivity::Private => private_warnings(path, &meta)?,
        Sensitivity::Public => Vec::new(),
    };
    Ok(Some(KeyFile { text, warnings }))
}

fn symlink(path: &Path) -> KeyError {
    KeyError::Symlink {
        path: path.to_path_buf(),
    }
}

fn not_regular(path: &Path) -> KeyError {
    KeyError::NotRegularFile {
        path: path.to_path_buf(),
    }
}

fn too_large(path: &Path) -> KeyError {
    KeyError::TooLarge {
        path: path.to_path_buf(),
        limit: MAX_KEY_FILE_LEN,
    }
}

/// Reads into a pre-sized, wiped buffer (no reallocation copies).
fn read_capped(file: File, path: &Path) -> Result<Zeroizing<String>, KeyError> {
    let mut text = Zeroizing::new(String::with_capacity(MAX_KEY_FILE_LEN + 1));
    file.take(MAX_KEY_FILE_LEN as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|err| KeyError::io("read", path, err))?;
    if text.len() > MAX_KEY_FILE_LEN {
        return Err(too_large(path));
    }
    Ok(text)
}

/// Opens without following a final symlink (closes the check/open race);
/// non-blocking so a swapped-in FIFO cannot hang startup.
#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<Option<File>, KeyError> {
    use rustix::fs::{Mode, OFlags};
    use rustix::io::Errno;

    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    match rustix::fs::open(path, flags, Mode::empty()) {
        Ok(fd) => Ok(Some(File::from(fd))),
        Err(err) if err == Errno::NOENT => Ok(None),
        Err(err) if err == Errno::LOOP => Err(symlink(path)),
        Err(err) => Err(KeyError::io("open", path, err.into())),
    }
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> Result<Option<File>, KeyError> {
    match File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(KeyError::io("open", path, err)),
    }
}

#[cfg(unix)]
fn private_warnings(path: &Path, meta: &std::fs::Metadata) -> Result<Vec<KeyWarning>, KeyError> {
    use std::os::unix::fs::MetadataExt;

    let dir = super::fs::parent_dir(path);
    let dir_mode = std::fs::metadata(&dir)
        .map_err(|err| KeyError::io("inspect", &dir, err))?
        .mode();
    let euid = rustix::process::geteuid().as_raw();
    Ok([
        mode_warning(path, meta.mode()),
        owner_warning(path, meta.uid(), euid),
        dir_warning(&dir, dir_mode),
    ]
    .into_iter()
    .flatten()
    .collect())
}

#[cfg(not(unix))]
fn private_warnings(_path: &Path, _meta: &std::fs::Metadata) -> Result<Vec<KeyWarning>, KeyError> {
    Ok(Vec::new())
}

#[cfg(unix)]
const PERMISSION_BITS: u32 = 0o7777;

/// Any group/other access to a private key file.
#[cfg(unix)]
fn mode_warning(path: &Path, mode: u32) -> Option<KeyWarning> {
    const GROUP_OTHER_ANY: u32 = 0o077;
    (mode & GROUP_OTHER_ANY != 0).then(|| KeyWarning::PermissiveMode {
        path: path.to_path_buf(),
        mode: mode & PERMISSION_BITS,
    })
}

#[cfg(unix)]
fn owner_warning(path: &Path, owner: u32, euid: u32) -> Option<KeyWarning> {
    (owner != euid).then(|| KeyWarning::ForeignOwner {
        path: path.to_path_buf(),
        owner,
        euid,
    })
}

/// Group/other write on the key directory lets others swap key files.
#[cfg(unix)]
fn dir_warning(dir: &Path, mode: u32) -> Option<KeyWarning> {
    const GROUP_OTHER_WRITE: u32 = 0o022;
    (mode & GROUP_OTHER_WRITE != 0).then(|| KeyWarning::WritableDirectory {
        path: dir.to_path_buf(),
        mode: mode & PERMISSION_BITS,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const FILE: &str = "/k/id_ed25519";

    #[test]
    fn owner_warning_only_for_foreign_uid() {
        assert_eq!(owner_warning(Path::new(FILE), 1000, 1000), None);
        let warning = owner_warning(Path::new(FILE), 0, 1000).unwrap();
        assert_eq!(
            warning,
            KeyWarning::ForeignOwner {
                path: PathBuf::from(FILE),
                owner: 0,
                euid: 1000
            }
        );
        assert!(warning.to_string().contains("uid 0"), "{warning}");
    }

    #[test]
    fn mode_warning_flags_any_group_or_other_bit() {
        assert_eq!(mode_warning(Path::new(FILE), 0o100600), None);
        assert_eq!(mode_warning(Path::new(FILE), 0o100400), None);
        for mode in [0o640, 0o604, 0o610, 0o601] {
            assert!(mode_warning(Path::new(FILE), mode).is_some(), "{mode:o}");
        }
    }

    #[test]
    fn dir_warning_flags_group_or_other_write_only() {
        assert_eq!(dir_warning(Path::new("/k"), 0o40700), None);
        assert_eq!(dir_warning(Path::new("/k"), 0o40755), None);
        assert_eq!(
            dir_warning(Path::new("/k"), 0o41777),
            Some(KeyWarning::WritableDirectory {
                path: PathBuf::from("/k"),
                mode: 0o1777
            })
        );
        assert!(dir_warning(Path::new("/k"), 0o770).is_some());
    }
}
