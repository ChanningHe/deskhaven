//! File-descriptor budget (review M-b, round 3 M2). Every connection costs a
//! descriptor; hitting `RLIMIT_NOFILE` makes `accept` fail and also breaks the
//! DB pool, files and the in-process signal server. HTTP therefore gets only a
//! share of the soft limit (`nofile_budget_percent`, default 50%), split 3:1 in
//! favour of the client API when the admin listener is separate.

/// Soft-limit target when the hard limit is unlimited (macOS): asking for
/// "infinity" fails there.
pub const NOFILE_TARGET_WHEN_UNLIMITED: u64 = 65_536;

const PERCENT: u64 = 100;
/// Split mode: client_api gets 3 parts, admin 1.
const CLIENT_API_PARTS: u64 = 3;
const ADMIN_PARTS: u64 = 1;

/// `RLIMIT_NOFILE` after [`raise_nofile_limit`]; `None` = unlimited/unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NofileInfo {
    pub soft: Option<u64>,
    pub hard: Option<u64>,
    /// The soft limit was raised by this call.
    pub raised: bool,
}

/// The soft limit to ask for, or `None` if the current one is already enough.
fn raise_target(soft: Option<u64>, hard: Option<u64>) -> Option<u64> {
    let soft = soft?; // already unlimited
    let target = hard.unwrap_or(NOFILE_TARGET_WHEN_UNLIMITED);
    (soft < target).then_some(target)
}

/// Raise the soft `RLIMIT_NOFILE` to the hard limit, or to
/// [`NOFILE_TARGET_WHEN_UNLIMITED`] when the hard limit is unlimited. Call once
/// at startup. Failure is logged and leaves the limit unchanged.
#[cfg(unix)]
pub fn raise_nofile_limit() -> NofileInfo {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

    let Rlimit { current, maximum } = getrlimit(Resource::Nofile);
    let unchanged = NofileInfo {
        soft: current,
        hard: maximum,
        raised: false,
    };
    let Some(target) = raise_target(current, maximum) else {
        return unchanged;
    };
    let wanted = Rlimit {
        current: Some(target),
        maximum,
    };
    match setrlimit(Resource::Nofile, wanted) {
        Ok(()) => NofileInfo {
            soft: Some(target),
            hard: maximum,
            raised: true,
        },
        Err(err) => {
            tracing::warn!(error = %err, target, "cannot raise RLIMIT_NOFILE soft limit");
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

/// Which listener a budget is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Share {
    /// The only listener.
    Single,
    SplitClientApi,
    SplitAdmin,
}

/// Connection cap for one listener: `configured`, clamped to its share of
/// `soft * percent / 100`. Never below 1.
pub(crate) fn connection_budget(
    configured: usize,
    soft: Option<u64>,
    percent: u8,
    share: Share,
) -> usize {
    let Some(soft) = soft else {
        return configured.max(1);
    };
    let http = soft.saturating_mul(u64::from(percent)) / PERCENT;
    let parts = CLIENT_API_PARTS + ADMIN_PARTS;
    let budget = match share {
        Share::Single => http,
        Share::SplitClientApi => http.saturating_mul(CLIENT_API_PARTS) / parts,
        Share::SplitAdmin => http.saturating_mul(ADMIN_PARTS) / parts,
    };
    let budget = usize::try_from(budget).unwrap_or(usize::MAX);
    configured.min(budget).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_keeps_configured_value() {
        assert_eq!(connection_budget(10_000, None, 50, Share::Single), 10_000);
    }

    #[test]
    fn single_listener_gets_the_http_share_of_the_soft_limit() {
        assert_eq!(
            connection_budget(10_000, Some(1024), 50, Share::Single),
            512
        );
        assert_eq!(
            connection_budget(10_000, Some(1024), 25, Share::Single),
            256
        );
    }

    #[test]
    fn split_listeners_share_three_to_one() {
        let client = connection_budget(10_000, Some(4096), 50, Share::SplitClientApi);
        let admin = connection_budget(10_000, Some(4096), 50, Share::SplitAdmin);
        assert_eq!((client, admin), (1536, 512));
        assert!(client + admin <= 4096 / 2);
    }

    #[test]
    fn configured_value_below_budget_is_kept() {
        assert_eq!(
            connection_budget(100, Some(65_536), 50, Share::SplitAdmin),
            100
        );
    }

    #[test]
    fn never_drops_below_one() {
        assert_eq!(connection_budget(10_000, Some(1), 50, Share::SplitAdmin), 1);
        assert_eq!(connection_budget(0, None, 50, Share::Single), 1);
    }

    #[test]
    fn raise_target_prefers_hard_limit() {
        assert_eq!(raise_target(Some(1024), Some(4096)), Some(4096));
        assert_eq!(raise_target(Some(4096), Some(4096)), None);
    }

    /// Review round 3 LOW: unlimited hard limit -> a sane finite target.
    #[test]
    fn raise_target_is_finite_when_hard_is_unlimited() {
        assert_eq!(
            raise_target(Some(256), None),
            Some(NOFILE_TARGET_WHEN_UNLIMITED)
        );
        assert_eq!(raise_target(Some(100_000), None), None);
        assert_eq!(raise_target(None, None), None);
    }

    #[cfg(unix)]
    #[test]
    fn raising_reports_the_limit_now_in_effect() {
        let info = raise_nofile_limit();
        assert_eq!(current_soft_limit(), info.soft);
        if let (Some(soft), Some(hard)) = (info.soft, info.hard) {
            assert!(soft <= hard);
        }
    }
}
