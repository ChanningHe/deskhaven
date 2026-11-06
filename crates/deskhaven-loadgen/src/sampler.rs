//! Samples the server process once a second, tagging each sample with the run phase.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;

use crate::procfs::{self, ProcSample};
use crate::stats::{Phase, Recorder};

const EVERY: Duration = Duration::from_secs(1);
/// Window of the "busiest seconds" CPU figure.
const PEAK_WINDOW: Duration = Duration::from_secs(10);

type Samples = Arc<Mutex<Vec<(Phase, ProcSample)>>>;

/// The sampling task; aborted if dropped before [`Sampler::finish`].
pub struct Sampler {
    samples: Samples,
    stop: CancellationToken,
    task: JoinSet<()>,
}

impl Sampler {
    pub fn start(pid: u32, recorder: Arc<Recorder>) -> Self {
        let samples = Samples::default();
        let stop = CancellationToken::new();
        let mut task = JoinSet::new();
        task.spawn(sample_until(
            pid,
            recorder,
            Arc::clone(&samples),
            stop.clone(),
        ));
        Self {
            samples,
            stop,
            task,
        }
    }

    pub async fn finish(mut self) -> Usage {
        self.stop.cancel();
        while self.task.join_next().await.is_some() {}
        let samples = self
            .samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        Usage::from_samples(&samples, procfs::ticks_per_second())
    }
}

async fn sample_until(
    pid: u32,
    recorder: Arc<Recorder>,
    samples: Samples,
    stop: CancellationToken,
) {
    let mut every = interval(EVERY);
    every.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut warned = false;
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = every.tick() => {}
        }
        match procfs::sample(pid) {
            Ok(sample) => {
                let phase = recorder.phase();
                let mut all = samples.lock().unwrap_or_else(PoisonError::into_inner);
                all.push((phase, sample));
            }
            Err(err) if !warned => {
                warned = true;
                tracing::warn!(pid, error = %err, "cannot sample the server process");
            }
            Err(_) => {}
        }
    }
}

/// Server resource use per phase.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub idle_rss_max: Option<u64>,
    pub steady_cores: Option<f64>,
    pub steady_peak_cores: Option<f64>,
    pub steady_rss_max: Option<u64>,
    pub steady_write_bytes: Option<u64>,
}

impl Usage {
    pub fn from_samples(samples: &[(Phase, ProcSample)], ticks_per_second: u64) -> Self {
        let in_phase = |phase: Phase| -> Vec<ProcSample> {
            samples
                .iter()
                .filter(|(p, _)| *p == phase)
                .map(|(_, s)| *s)
                .collect()
        };
        let idle = in_phase(Phase::Idle);
        let steady = in_phase(Phase::Steady);
        let edges = steady
            .first()
            .zip(steady.last())
            .filter(|(a, b)| a.at < b.at);
        Self {
            idle_rss_max: idle.iter().map(|s| s.rss_bytes).max(),
            steady_cores: edges.map(|(a, b)| procfs::cores_between(a, b, ticks_per_second)),
            steady_peak_cores: procfs::peak_cores(&steady, PEAK_WINDOW, ticks_per_second),
            steady_rss_max: steady.iter().map(|s| s.rss_bytes).max(),
            steady_write_bytes: edges
                .and_then(|(a, b)| Some(b.write_bytes?.saturating_sub(a.write_bytes?))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_is_computed_per_phase() {
        // Arrange
        let base = std::time::Instant::now();
        let sample = |secs: u64, cpu_ticks: u64, rss_mib: u64, written: u64| ProcSample {
            at: base + Duration::from_secs(secs),
            cpu_ticks,
            rss_bytes: rss_mib << 20,
            write_bytes: Some(written),
        };
        let samples = vec![
            (Phase::Idle, sample(0, 0, 20, 0)),
            (Phase::Idle, sample(1, 1, 22, 0)),
            (Phase::Ramp, sample(2, 100, 60, 500)),
            (Phase::Steady, sample(3, 200, 40, 1_000)),
            (Phase::Steady, sample(5, 260, 45, 4_000)),
        ];

        // Act
        let usage = Usage::from_samples(&samples, 100);

        // Assert
        assert_eq!(usage.idle_rss_max, Some(22 << 20));
        assert_eq!(usage.steady_rss_max, Some(45 << 20));
        assert!((usage.steady_cores.unwrap() - 0.3).abs() < 1e-9);
        assert_eq!(usage.steady_write_bytes, Some(3_000));
    }

    #[test]
    fn a_single_steady_sample_gives_no_rate() {
        let base = std::time::Instant::now();
        let one = ProcSample {
            at: base,
            cpu_ticks: 5,
            rss_bytes: 1,
            write_bytes: None,
        };

        let usage = Usage::from_samples(&[(Phase::Steady, one)], 100);

        assert_eq!(usage.steady_cores, None);
        assert_eq!(usage.steady_write_bytes, None);
        assert_eq!(usage.steady_rss_max, Some(1));
    }
}
