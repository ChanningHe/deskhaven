//! One server process per data_dir: an exclusive, non-blocking advisory lock
//! (`flock`) on `{data_dir}/deskhaven.lock`, held for the process lifetime.
//! The kernel drops it when the process exits, so a crash never leaves a
//! stale lock behind. Two instances on separate data_dirs sharing one
//! PostgreSQL database are not detected.

use std::path::Path;

/// Lock file name inside data_dir; its content is unused.
pub const LOCK_FILE: &str = "deskhaven.lock";

/// Held lock; released on drop.
#[derive(Debug)]
pub struct InstanceLock {
    #[cfg(unix)]
    _fd: rustix::fd::OwnedFd,
}

/// Takes the lock, or fails at once if another process holds it.
#[cfg(unix)]
pub fn acquire(data_dir: &Path) -> anyhow::Result<InstanceLock> {
    use anyhow::{Context, bail};
    use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
    use rustix::io::Errno;

    const LOCK_FILE_MODE: u32 = 0o600;
    let path = data_dir.join(LOCK_FILE);
    let flags = OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let fd = open(&path, flags, Mode::from_raw_mode(LOCK_FILE_MODE))
        .map_err(std::io::Error::from)
        .with_context(|| format!("cannot open the lock file {}", path.display()))?;
    match flock(&fd, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(InstanceLock { _fd: fd }),
        Err(Errno::WOULDBLOCK) => {
            bail!("another deskhaven instance is using {}", data_dir.display())
        }
        Err(err) => Err(std::io::Error::from(err))
            .with_context(|| format!("cannot lock {}", path.display())),
    }
}

#[cfg(not(unix))]
pub fn acquire(_data_dir: &Path) -> anyhow::Result<InstanceLock> {
    Ok(InstanceLock {})
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn second_lock_fails_until_the_first_is_dropped() {
        let dir = tempfile::tempdir().unwrap();

        let first = acquire(dir.path()).unwrap();
        let err = acquire(dir.path()).unwrap_err();
        drop(first);
        let again = acquire(dir.path());

        assert!(
            err.to_string()
                .contains("another deskhaven instance is using"),
            "{err}"
        );
        assert!(again.is_ok(), "{again:?}");
    }

    #[test]
    fn refuses_a_symlinked_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/dev/null", dir.path().join(LOCK_FILE)).unwrap();

        assert!(acquire(dir.path()).is_err());
    }
}
