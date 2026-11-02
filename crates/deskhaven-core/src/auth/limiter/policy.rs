//! The account lockout state machine, free of I/O. All times are unix milliseconds.
//!
//! - Failures are counted in a fixed window that opens at the first failure and lasts
//!   `auth.lockout.window`; a failure after the window closed starts a new one at 1.
//! - Reaching `auth.lockout.max_failures` in one window locks the key for
//!   `auth.lockout.duration × 2^level`, capped at [`MAX_LOCK`]; the counter and window
//!   restart at the lock.
//! - The level is 0 for the first lock, and one more than the previous lock's level when
//!   that lock ended less than [`LEVEL_DECAY`] ago. After a quiet period of [`LEVEL_DECAY`]
//!   with no lock in force, the next lock starts at level 0 again.
//! - Failures while locked change nothing (they normally never get here: the login
//!   pre-check rejects locked keys before authenticating).
//! - A successful login deletes the row (failures and level reset).

use std::time::Duration;

use deskhaven_config::settings::Snapshot;
use deskhaven_config::settings::{
    AUTH_LOCKOUT_DURATION, AUTH_LOCKOUT_MAX_FAILURES, AUTH_LOCKOUT_WINDOW,
};

/// Upper bound of one lock (05-config §3.6).
pub const MAX_LOCK: Duration = Duration::from_secs(24 * 60 * 60);
/// Quiet period after which the backoff level resets.
pub const LEVEL_DECAY: Duration = Duration::from_secs(24 * 60 * 60);
/// Highest stored level. `duration` is at least 1 minute, and 1m × 2^11 already exceeds
/// [`MAX_LOCK`], so higher levels would not change anything.
const MAX_LEVEL: i32 = 11;

/// The lockout settings, read from one snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Policy {
    pub max_failures: i32,
    pub window_ms: i64,
    pub base_lock_ms: i64,
}

impl Policy {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            max_failures: i32::try_from(snapshot.get(&AUTH_LOCKOUT_MAX_FAILURES))
                .unwrap_or(i32::MAX),
            window_ms: millis(snapshot.get(&AUTH_LOCKOUT_WINDOW).as_std()),
            base_lock_ms: millis(snapshot.get(&AUTH_LOCKOUT_DURATION).as_std()),
        }
    }

    /// The state after one more failure at `now`.
    pub fn on_failure(&self, state: State, now: i64) -> State {
        if state.is_locked(now) {
            return state;
        }
        let window_open = now.saturating_sub(state.window_started_at) < self.window_ms;
        let (failures, window_started_at) = if window_open {
            (state.failures.saturating_add(1), state.window_started_at)
        } else {
            (1, now)
        };
        if failures < self.max_failures {
            return State {
                failures,
                window_started_at,
                ..state
            };
        }
        let level = match state.locked_until {
            Some(until) if now.saturating_sub(until) < millis(LEVEL_DECAY) => {
                (state.lock_level + 1).min(MAX_LEVEL)
            }
            _ => 0,
        };
        State {
            failures: 0,
            window_started_at: now,
            locked_until: Some(now.saturating_add(self.lock_ms(level))),
            lock_level: level,
        }
    }

    /// `duration × 2^level`, capped at [`MAX_LOCK`].
    pub fn lock_ms(&self, level: i32) -> i64 {
        let factor = 1_i64 << level.clamp(0, MAX_LEVEL);
        self.base_lock_ms
            .saturating_mul(factor)
            .min(millis(MAX_LOCK))
    }
}

/// The decision-relevant columns of an `auth_lockouts` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct State {
    pub failures: i32,
    pub window_started_at: i64,
    pub locked_until: Option<i64>,
    pub lock_level: i32,
}

impl State {
    /// No failures yet.
    pub fn fresh(now: i64) -> Self {
        Self {
            failures: 0,
            window_started_at: now,
            locked_until: None,
            lock_level: 0,
        }
    }

    pub fn is_locked(&self, now: i64) -> bool {
        self.locked_until.is_some_and(|until| until > now)
    }

    /// Time left on the lock at `now`, if locked.
    pub fn retry_after(&self, now: i64) -> Option<Duration> {
        let until = self.locked_until.filter(|until| *until > now)?;
        Some(Duration::from_millis(
            u64::try_from(until - now).unwrap_or(u64::MAX),
        ))
    }
}

pub(super) fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const T0: i64 = 1_700_000_000_000;

    fn policy() -> Policy {
        Policy {
            max_failures: 3,
            window_ms: 15 * MIN,
            base_lock_ms: 15 * MIN,
        }
    }

    fn fail_n(p: &Policy, mut s: State, now: i64, n: usize) -> State {
        for _ in 0..n {
            s = p.on_failure(s, now);
        }
        s
    }

    #[test]
    fn locks_on_the_max_failure_within_the_window() {
        let p = policy();
        let s = fail_n(&p, State::fresh(T0), T0, 2);
        assert_eq!(s.failures, 2);
        assert!(!s.is_locked(T0));

        let s = p.on_failure(s, T0 + MIN);
        assert_eq!(s.locked_until, Some(T0 + MIN + 15 * MIN));
        assert_eq!((s.failures, s.lock_level), (0, 0));
        assert_eq!(s.retry_after(T0 + MIN), Some(Duration::from_secs(15 * 60)));
    }

    #[test]
    fn a_closed_window_restarts_the_count() {
        let p = policy();
        let s = fail_n(&p, State::fresh(T0), T0, 2);
        let s = p.on_failure(s, T0 + 15 * MIN);
        assert_eq!((s.failures, s.window_started_at), (1, T0 + 15 * MIN));
        assert!(!s.is_locked(T0 + 15 * MIN));
    }

    #[test]
    fn failures_while_locked_change_nothing() {
        let p = policy();
        let locked = fail_n(&p, State::fresh(T0), T0, 3);
        assert_eq!(p.on_failure(locked, T0 + MIN), locked);
    }

    #[test]
    fn repeated_locks_double_up_to_the_cap() {
        let p = policy();
        let mut s = State::fresh(T0);
        let mut now = T0;
        let mut seen = Vec::new();
        for _ in 0..9 {
            s = fail_n(&p, s, now, 3);
            let until = s.locked_until.unwrap_or_default();
            seen.push((until - now) / MIN);
            now = until;
        }
        assert_eq!(seen, [15, 30, 60, 120, 240, 480, 960, 1440, 1440]);
        assert_eq!(s.lock_level, 8);
    }

    #[test]
    fn level_resets_after_a_quiet_period() {
        let p = policy();
        let s = fail_n(&p, State::fresh(T0), T0, 3);
        let ended = s.locked_until.unwrap_or_default();

        let soon = fail_n(&p, s, ended + millis(LEVEL_DECAY) - 1, 3);
        assert_eq!(soon.lock_level, 1);

        let later = fail_n(&p, s, ended + 24 * HOUR, 3);
        assert_eq!(later.lock_level, 0);
    }

    #[test]
    fn lock_ms_saturates_and_clamps() {
        let p = Policy {
            base_lock_ms: 24 * HOUR,
            ..policy()
        };
        assert_eq!(p.lock_ms(0), 24 * HOUR);
        assert_eq!(p.lock_ms(i32::MAX), 24 * HOUR);
        assert_eq!(policy().lock_ms(-5), 15 * MIN);
    }

    #[test]
    fn max_failures_of_one_locks_on_the_first_failure() {
        let p = Policy {
            max_failures: 1,
            ..policy()
        };
        assert!(p.on_failure(State::fresh(T0), T0).is_locked(T0));
    }
}
