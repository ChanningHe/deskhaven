//! The lockout state machine, free of I/O. All times are unix milliseconds.
//!
//! Attempts are counted **before** verification (the login pre-check reserves one), so a
//! reservation is an attempt, not yet a failure; a success deletes the rows and a refund
//! (verification never ran) gives the attempt back.
//!
//! - Attempts are counted in a fixed window that opens at the first one and lasts
//!   `auth.lockout.window`; an attempt after the window closed starts a new one at 1.
//! - Reaching the threshold in one window locks the key; the counter and window restart at
//!   the lock. The attempt that reaches the threshold is still verified, so at most
//!   `threshold` verifications happen per lock cycle.
//! - Per-source rows ([`Policy::per_source`]): threshold `auth.lockout.max_failures`, lock
//!   `auth.lockout.duration × 2^level`, capped at [`MAX_LOCK`]. The level is 0 for the first
//!   lock, and one more than the previous lock's level when that lock ended less than
//!   [`LEVEL_DECAY`] ago; after a quiet period of [`LEVEL_DECAY`] with no lock in force it
//!   starts at 0 again.
//! - Global rows ([`Policy::global`]): threshold [`GLOBAL_FACTOR`] × `max_failures`, fixed
//!   lock of `auth.lockout.duration` (no backoff), so distributed guessing is bounded but no
//!   single source can lock the account for long.
//! - Attempts while locked change nothing (the pre-check rejects them).

use std::time::Duration;

use deskhaven_config::settings::Snapshot;
use deskhaven_config::settings::{
    AUTH_LOCKOUT_DURATION, AUTH_LOCKOUT_MAX_FAILURES, AUTH_LOCKOUT_WINDOW, LOCKOUT_GLOBAL_FACTOR,
};

/// Upper bound of one lock (05-config §3.6).
pub const MAX_LOCK: Duration = Duration::from_secs(24 * 60 * 60);
/// Quiet period after which the backoff level resets.
pub const LEVEL_DECAY: Duration = Duration::from_secs(24 * 60 * 60);
/// The global row's threshold is this many times the per-source one. Shared with the
/// settings rule `auth.lockout.window <= 4 × auth.lockout.duration`.
#[allow(clippy::cast_possible_wrap)] // 4
pub const GLOBAL_FACTOR: i32 = LOCKOUT_GLOBAL_FACTOR as i32;
/// Highest stored level. `duration` is at least 1 minute, and 1m × 2^11 already exceeds
/// [`MAX_LOCK`], so higher levels would not change anything.
const MAX_LEVEL: i32 = 11;

/// The lockout rule of one kind of row, read from one settings snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Policy {
    pub threshold: i32,
    pub window_ms: i64,
    pub base_lock_ms: i64,
    pub backoff: bool,
}

impl Policy {
    /// `user:<name>|<source>` rows: exponential backoff.
    pub fn per_source(snapshot: &Snapshot) -> Self {
        Self {
            threshold: i32::try_from(snapshot.get(&AUTH_LOCKOUT_MAX_FAILURES)).unwrap_or(i32::MAX),
            window_ms: millis(snapshot.get(&AUTH_LOCKOUT_WINDOW).as_std()),
            base_lock_ms: millis(snapshot.get(&AUTH_LOCKOUT_DURATION).as_std()),
            backoff: true,
        }
    }

    /// `user:<name>|*` rows: higher threshold, fixed duration.
    pub fn global(snapshot: &Snapshot) -> Self {
        let per_source = Self::per_source(snapshot);
        Self {
            threshold: per_source.threshold.saturating_mul(GLOBAL_FACTOR),
            backoff: false,
            ..per_source
        }
    }

    /// The state after one more attempt at `now`.
    pub fn on_attempt(&self, state: State, now: i64) -> State {
        if state.is_locked(now) {
            return state;
        }
        let window_open = now.saturating_sub(state.window_started_at) < self.window_ms;
        let (failures, window_started_at) = if window_open {
            (state.failures.saturating_add(1), state.window_started_at)
        } else {
            (1, now)
        };
        if failures < self.threshold {
            return State {
                failures,
                window_started_at,
                ..state
            };
        }
        let level = match state.locked_until {
            Some(until) if self.backoff && now.saturating_sub(until) < millis(LEVEL_DECAY) => {
                state.lock_level.saturating_add(1).min(MAX_LEVEL)
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
    /// No attempts yet.
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

    /// Whether going from `self` to `next` started a lock.
    pub fn starts_lock(&self, next: &State) -> bool {
        next.locked_until.is_some() && next.locked_until != self.locked_until
    }
}

/// One reserved attempt on one row: the row before and after the reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Reservation {
    pub before: State,
    pub after: State,
}

impl Reservation {
    /// The row with this attempt given back, or `None` if there is nothing to give back.
    /// Untouched since the reservation: exactly the state before it (a lock it started is
    /// lifted). Otherwise best effort: one attempt less, if any are counted.
    pub fn refund(&self, current: State) -> Option<State> {
        if current == self.after {
            return Some(self.before);
        }
        (current.failures > 0).then(|| State {
            failures: current.failures - 1,
            ..current
        })
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
            threshold: 3,
            window_ms: 15 * MIN,
            base_lock_ms: 15 * MIN,
            backoff: true,
        }
    }

    fn attempt_n(p: &Policy, mut s: State, now: i64, n: usize) -> State {
        for _ in 0..n {
            s = p.on_attempt(s, now);
        }
        s
    }

    #[test]
    fn locks_on_the_threshold_attempt_within_the_window() {
        let p = policy();
        let s = attempt_n(&p, State::fresh(T0), T0, 2);
        assert_eq!(s.failures, 2);
        assert!(!s.is_locked(T0));

        let s = p.on_attempt(s, T0 + MIN);
        assert_eq!(s.locked_until, Some(T0 + MIN + 15 * MIN));
        assert_eq!((s.failures, s.lock_level), (0, 0));
        assert_eq!(s.retry_after(T0 + MIN), Some(Duration::from_secs(15 * 60)));
    }

    #[test]
    fn a_closed_window_restarts_the_count() {
        let p = policy();
        let s = attempt_n(&p, State::fresh(T0), T0, 2);
        let s = p.on_attempt(s, T0 + 15 * MIN);
        assert_eq!((s.failures, s.window_started_at), (1, T0 + 15 * MIN));
        assert!(!s.is_locked(T0 + 15 * MIN));
    }

    #[test]
    fn attempts_while_locked_change_nothing() {
        let p = policy();
        let locked = attempt_n(&p, State::fresh(T0), T0, 3);
        assert_eq!(p.on_attempt(locked, T0 + MIN), locked);
    }

    #[test]
    fn repeated_locks_double_up_to_the_cap() {
        let p = policy();
        let mut s = State::fresh(T0);
        let mut now = T0;
        let mut seen = Vec::new();
        for _ in 0..9 {
            s = attempt_n(&p, s, now, 3);
            let until = s.locked_until.unwrap_or_default();
            seen.push((until - now) / MIN);
            now = until;
        }
        assert_eq!(seen, [15, 30, 60, 120, 240, 480, 960, 1440, 1440]);
        assert_eq!(s.lock_level, 8);
    }

    #[test]
    fn level_saturates_at_the_maximum() {
        let p = policy();
        let s = State {
            locked_until: Some(T0),
            lock_level: i32::MAX,
            ..State::fresh(T0)
        };
        assert_eq!(attempt_n(&p, s, T0, 3).lock_level, MAX_LEVEL);
    }

    #[test]
    fn global_policy_has_a_higher_threshold_and_no_backoff() {
        let snapshot = Snapshot::defaults();
        let per_source = Policy::per_source(&snapshot);
        let global = Policy::global(&snapshot);
        assert_eq!(global.threshold, per_source.threshold * GLOBAL_FACTOR);

        let mut s = State::fresh(T0);
        let mut now = T0;
        for _ in 0..3 {
            s = attempt_n(
                &global,
                s,
                now,
                usize::try_from(global.threshold).unwrap_or(0),
            );
            assert_eq!(s.locked_until, Some(now + per_source.base_lock_ms));
            assert_eq!(s.lock_level, 0);
            now = s.locked_until.unwrap_or_default();
        }
    }

    #[test]
    fn level_resets_after_a_quiet_period() {
        let p = policy();
        let s = attempt_n(&p, State::fresh(T0), T0, 3);
        let ended = s.locked_until.unwrap_or_default();

        let soon = attempt_n(&p, s, ended + millis(LEVEL_DECAY) - 1, 3);
        assert_eq!(soon.lock_level, 1);

        let later = attempt_n(&p, s, ended + 24 * HOUR, 3);
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
    fn threshold_of_one_locks_on_the_first_attempt() {
        let p = Policy {
            threshold: 1,
            ..policy()
        };
        assert!(p.on_attempt(State::fresh(T0), T0).is_locked(T0));
    }

    #[test]
    fn refund_restores_an_untouched_row_exactly() {
        let p = policy();
        let before = attempt_n(&p, State::fresh(T0), T0, 2);
        let after = p.on_attempt(before, T0);
        assert!(after.is_locked(T0));
        let r = Reservation { before, after };

        assert_eq!(r.refund(after), Some(before));
    }

    #[test]
    fn refund_of_a_changed_row_takes_back_one_attempt() {
        let p = policy();
        let before = State::fresh(T0);
        let after = p.on_attempt(before, T0);
        let r = Reservation { before, after };

        let later = p.on_attempt(after, T0);
        assert_eq!(r.refund(later).map(|s| s.failures), Some(1));
        assert_eq!(r.refund(State::fresh(T0)), None);
    }

    #[test]
    fn starts_lock_detects_only_new_locks() {
        let p = policy();
        let two = attempt_n(&p, State::fresh(T0), T0, 2);
        let locked = p.on_attempt(two, T0);
        assert!(two.starts_lock(&locked));
        assert!(!locked.starts_lock(&locked));
        assert!(!State::fresh(T0).starts_lock(&two));
    }
}
