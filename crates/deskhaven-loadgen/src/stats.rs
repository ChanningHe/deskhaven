//! Client-observed outcomes of the simulated requests, per run phase and request kind.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// Where the run is; every sample is filed under the phase it completed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Phase {
    /// Server up, no devices yet (idle RSS / CPU).
    Idle = 0,
    /// Devices start, spread over the ramp (first registration and sysinfo of each).
    Ramp = 1,
    /// Every device started; stragglers finish their first round.
    Settle = 2,
    /// The measurement window the NFRs are judged on.
    Steady = 3,
    /// Devices stopping.
    Drain = 4,
}

impl Phase {
    fn from_u8(raw: u8) -> Self {
        match raw {
            0 => Self::Idle,
            1 => Self::Ramp,
            2 => Self::Settle,
            3 => Self::Steady,
            _ => Self::Drain,
        }
    }
}

/// What a simulated device sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Heartbeat,
    Sysinfo,
    Audit,
    RegisterPeer,
    RegisterPk,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Heartbeat => "POST /api/heartbeat",
            Self::Sysinfo => "POST /api/sysinfo",
            Self::Audit => "POST /api/audit/conn",
            Self::RegisterPeer => "RegisterPeer (UDP)",
            Self::RegisterPk => "RegisterPk (UDP)",
        })
    }
}

/// Why a request did not get the answer a client counts as success.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Failure {
    /// An HTTP status other than the expected one.
    Status(u16),
    /// The expected status with a body the client would not accept.
    Body(String),
    /// No (complete) answer within the request timeout.
    Timeout,
    /// Socket or protocol error.
    Transport(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(code) => write!(f, "HTTP {code}"),
            Self::Body(what) => write!(f, "body: {what}"),
            Self::Timeout => f.write_str("timeout"),
            Self::Transport(what) => write!(f, "transport: {what}"),
        }
    }
}

/// Nearest-rank percentile (`p` in 0–100) of ascending `sorted` samples; `None` if empty.
pub fn percentile(sorted: &[Duration], p: f64) -> Option<Duration> {
    let last = sorted.len().checked_sub(1)?;
    // Rank in 1..=len; the casts are exact for any realistic sample count.
    let rank = (p.clamp(0.0, 100.0) / 100.0 * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.saturating_sub(1).min(last)).copied()
}

/// Latency distribution and failure counts of one series.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub ok: u64,
    pub failures: BTreeMap<Failure, u64>,
    pub p50: Option<Duration>,
    pub p90: Option<Duration>,
    pub p99: Option<Duration>,
    pub max: Option<Duration>,
}

impl Summary {
    pub fn failed(&self) -> u64 {
        self.failures.values().sum()
    }
}

#[derive(Default)]
struct Series {
    /// Latencies of successful requests.
    samples: Vec<Duration>,
    failures: BTreeMap<Failure, u64>,
}

impl Series {
    fn summary(&self) -> Summary {
        let mut sorted = self.samples.clone();
        sorted.sort_unstable();
        Summary {
            ok: u64::try_from(sorted.len()).unwrap_or(u64::MAX),
            failures: self.failures.clone(),
            p50: percentile(&sorted, 50.0),
            p90: percentile(&sorted, 90.0),
            p99: percentile(&sorted, 99.0),
            max: sorted.last().copied(),
        }
    }
}

/// Shared by every device task.
#[derive(Default)]
pub struct Recorder {
    phase: AtomicU8,
    series: Mutex<HashMap<(Phase, Kind), Series>>,
}

impl Recorder {
    pub fn phase(&self) -> Phase {
        Phase::from_u8(self.phase.load(Ordering::Relaxed))
    }

    pub fn set_phase(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Relaxed);
    }

    /// Files one finished request under the current phase.
    pub fn record(&self, kind: Kind, latency: Duration, result: Result<(), Failure>) {
        let phase = self.phase();
        let mut series = self.lock();
        let entry = series.entry((phase, kind)).or_default();
        match result {
            Ok(()) => entry.samples.push(latency),
            Err(failure) => *entry.failures.entry(failure).or_default() += 1,
        }
    }

    pub fn summary(&self, phase: Phase, kind: Kind) -> Summary {
        self.lock()
            .get(&(phase, kind))
            .map(Series::summary)
            .unwrap_or_default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<(Phase, Kind), Series>> {
        // Pushing a sample cannot leave the map inconsistent.
        self.series.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(values: &[u64]) -> Vec<Duration> {
        values.iter().copied().map(Duration::from_millis).collect()
    }

    #[test]
    fn percentile_of_nothing_is_none() {
        assert_eq!(percentile(&[], 50.0), None);
    }

    #[test]
    fn percentile_uses_the_nearest_rank() {
        // Arrange: 1..=100 ms.
        let sorted: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();

        // Act + Assert
        assert_eq!(percentile(&sorted, 50.0), Some(Duration::from_millis(50)));
        assert_eq!(percentile(&sorted, 99.0), Some(Duration::from_millis(99)));
        assert_eq!(percentile(&sorted, 99.5), Some(Duration::from_millis(100)));
        assert_eq!(percentile(&sorted, 100.0), Some(Duration::from_millis(100)));
        assert_eq!(percentile(&sorted, 0.0), Some(Duration::from_millis(1)));
    }

    #[test]
    fn percentile_of_few_samples_rounds_up_to_a_real_sample() {
        let sorted = ms(&[10, 20, 30]);

        assert_eq!(percentile(&sorted, 50.0), Some(Duration::from_millis(20)));
        assert_eq!(percentile(&sorted, 99.0), Some(Duration::from_millis(30)));
        assert_eq!(percentile(&sorted, 1.0), Some(Duration::from_millis(10)));
    }

    #[test]
    fn recorder_files_samples_under_the_current_phase() {
        // Arrange
        let recorder = Recorder::default();
        recorder.set_phase(Phase::Ramp);
        recorder.record(Kind::Heartbeat, Duration::from_millis(5), Ok(()));
        recorder.set_phase(Phase::Steady);

        // Act
        for latency in [3, 1, 2] {
            recorder.record(Kind::Heartbeat, Duration::from_millis(latency), Ok(()));
        }
        recorder.record(Kind::Heartbeat, Duration::ZERO, Err(Failure::Status(404)));
        recorder.record(Kind::Heartbeat, Duration::ZERO, Err(Failure::Status(404)));
        recorder.record(Kind::Sysinfo, Duration::ZERO, Err(Failure::Timeout));

        // Assert
        let steady = recorder.summary(Phase::Steady, Kind::Heartbeat);
        assert_eq!(steady.ok, 3);
        assert_eq!(steady.p50, Some(Duration::from_millis(2)));
        assert_eq!(steady.max, Some(Duration::from_millis(3)));
        assert_eq!(steady.failures.get(&Failure::Status(404)), Some(&2));
        assert_eq!(steady.failed(), 2);
        assert_eq!(recorder.summary(Phase::Ramp, Kind::Heartbeat).ok, 1);
        assert_eq!(recorder.summary(Phase::Steady, Kind::Sysinfo).failed(), 1);
        assert_eq!(
            recorder.summary(Phase::Idle, Kind::Audit),
            Summary::default()
        );
    }
}
