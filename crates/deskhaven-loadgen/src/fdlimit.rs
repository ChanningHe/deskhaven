//! Open-file limit preflight. Each device holds a UDP socket for the whole run and at most
//! one TCP connection at a time, so N devices need about 2N descriptors. A soft limit below
//! that is raised up to the hard limit (no privileges needed); if even the hard limit is too
//! low the run fails before starting the server, instead of failing devices mid-run.

use crate::LoadgenError;

/// Descriptors for everything besides the devices: runtime, database probe, logs, stdio.
const RESERVE: u64 = 256;

/// Descriptors `devices` simulated devices need.
pub fn needed(devices: usize) -> u64 {
    u64::try_from(devices)
        .unwrap_or(u64::MAX)
        .saturating_mul(2)
        .saturating_add(RESERVE)
}

/// The soft limit to set (`None`: the current one suffices) for `soft` / `hard` limits
/// (`None` = unlimited), or why it cannot work.
pub fn plan(soft: Option<u64>, hard: Option<u64>, needed: u64) -> Result<Option<u64>, String> {
    let enough = |limit: Option<u64>| limit.is_none_or(|l| l >= needed);
    match (enough(soft), enough(hard)) {
        (true, _) => Ok(None),
        (false, true) => Ok(Some(needed)),
        (false, false) => Err(format!(
            "the devices need {needed} open files but the hard limit is {}; raise it \
             (e.g. `ulimit -n {needed}` as a user allowed to) or run fewer devices",
            hard.unwrap_or_default()
        )),
    }
}

/// Applies [`plan`] to `RLIMIT_NOFILE` of this process (inherited by the server it starts).
#[cfg(unix)]
pub fn ensure(devices: usize) -> Result<(), LoadgenError> {
    use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
    let current = getrlimit(Resource::Nofile);
    let want = needed(devices);
    let Some(soft) = plan(current.current, current.maximum, want).map_err(LoadgenError::Config)?
    else {
        return Ok(());
    };
    let raised = Rlimit {
        current: Some(soft),
        maximum: current.maximum,
    };
    setrlimit(Resource::Nofile, raised).map_err(|err| {
        LoadgenError::Config(format!("cannot raise the open-file limit to {soft}: {err}"))
    })?;
    tracing::info!(soft, "raised the open-file soft limit");
    Ok(())
}

#[cfg(not(unix))]
pub fn ensure(_devices: usize) -> Result<(), LoadgenError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_descriptors_per_device_plus_a_reserve() {
        assert_eq!(needed(10_000), 20_256);
        assert_eq!(needed(1), 258);
    }

    #[test]
    fn a_sufficient_soft_limit_is_kept() {
        assert_eq!(plan(Some(65_536), Some(65_536), 20_256), Ok(None));
        assert_eq!(plan(None, None, 20_256), Ok(None), "unlimited");
    }

    #[test]
    fn a_low_soft_limit_is_raised_to_what_is_needed() {
        assert_eq!(plan(Some(1_024), Some(524_288), 20_256), Ok(Some(20_256)));
        assert_eq!(plan(Some(1_024), None, 20_256), Ok(Some(20_256)));
    }

    #[test]
    fn a_low_hard_limit_fails_with_the_numbers() {
        let err = plan(Some(1_024), Some(4_096), 20_256).unwrap_err();

        assert!(err.contains("20256") && err.contains("4096"), "{err}");
        assert!(err.contains("ulimit"), "says how to fix it: {err}");
    }
}
