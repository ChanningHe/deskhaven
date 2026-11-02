//! File-descriptor budget (review M-b). Every connection costs a descriptor;
//! hitting `RLIMIT_NOFILE` makes `accept` fail and also breaks DB, files and the
//! signal server, so connection caps are kept below it.

/// Descriptors kept free for the DB pool, files, logs and the signal server.
pub const NOFILE_RESERVE: u64 = 256;

/// `RLIMIT_NOFILE` after [`raise_nofile_limit`]; `None` = unlimited/unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NofileInfo {
    pub soft: Option<u64>,
    pub hard: Option<u64>,
    /// The soft limit was raised by this call.
    pub raised: bool,
}

/// Raise the soft `RLIMIT_NOFILE` to the hard limit (call once at startup).
/// Failure is logged and leaves the limit unchanged.
#[cfg(unix)]
pub fn raise_nofile_limit() -> NofileInfo {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

    let Rlimit { current, maximum } = getrlimit(Resource::Nofile);
    let unchanged = NofileInfo {
        soft: current,
        hard: maximum,
        raised: false,
    };
    if current == maximum {
        return unchanged;
    }
    let target = Rlimit {
        current: maximum,
        maximum,
    };
    match setrlimit(Resource::Nofile, target) {
        Ok(()) => NofileInfo {
            soft: maximum,
            hard: maximum,
            raised: true,
        },
        Err(err) => {
            tracing::warn!(error = %err, "cannot raise RLIMIT_NOFILE soft limit");
            unchanged
        }
    }
}

#[cfg(not(unix))]
pub fn raise_nofile_limit() -> NofileInfo {
    NofileInfo {
        soft: None,
        hard: None,
        raised: false,
    }
}

#[cfg(unix)]
pub(crate) fn current_soft_limit() -> Option<u64> {
    rustix::process::getrlimit(rustix::process::Resource::Nofile).current
}

#[cfg(not(unix))]
pub(crate) fn current_soft_limit() -> Option<u64> {
    None
}

/// Per-listener connection cap that fits `listeners` listeners into the
/// descriptor budget `soft - NOFILE_RESERVE`. Never below 1.
pub(crate) fn connection_budget(configured: usize, soft: Option<u64>, listeners: usize) -> usize {
    let Some(soft) = soft else {
        return configured.max(1);
    };
    let per_listener = soft.saturating_sub(NOFILE_RESERVE) / listeners.max(1) as u64;
    let per_listener = usize::try_from(per_listener).unwrap_or(usize::MAX);
    configured.min(per_listener).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_keeps_configured_value() {
        assert_eq!(connection_budget(10_000, None, 2), 10_000);
    }

    #[test]
    fn clamps_to_soft_limit_minus_reserve() {
        assert_eq!(connection_budget(10_000, Some(1024), 1), 768);
    }

    #[test]
    fn splits_budget_between_listeners() {
        assert_eq!(connection_budget(10_000, Some(1024), 2), 384);
    }

    #[test]
    fn configured_value_below_budget_is_kept() {
        assert_eq!(connection_budget(100, Some(65_536), 2), 100);
    }

    #[test]
    fn never_drops_below_one() {
        assert_eq!(connection_budget(10_000, Some(100), 1), 1);
        assert_eq!(connection_budget(0, None, 1), 1);
    }

    #[cfg(unix)]
    #[test]
    fn raising_sets_soft_to_hard_when_possible() {
        let info = raise_nofile_limit();
        assert_eq!(current_soft_limit(), info.soft);
        if let (Some(soft), Some(hard)) = (info.soft, info.hard) {
            assert_eq!(soft, hard);
        }
    }
}
