//! [`Scheduler`]: runs each [`Job`] in its own task, every [`Job::interval`], until the
//! shutdown token is cancelled; [`Scheduler::finish`] then lets each job do its last work
//! (the final `last_seen_at` flush) once the rest of the server has stopped.
//!
//! - **Independent**: one task per job, so a long retention purge never delays the
//!   `last_seen_at` flush.
//! - **Never dies**: every call into a job (its waits, runs and finish) is guarded; a panic
//!   is logged with its message and the job goes on at its next tick, like a run that
//!   failed (jobs log their own errors). So the loop always hands the job back and its
//!   [`Job::finish`] (the final flush) always runs.
//! - **Shutdown**: cancellation drops a run at its next await; jobs are cancel-safe (see
//!   `deskhaven_core::jobs`).

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// The wait used when a job's [`Job::interval`] or [`Job::first_delay`] panicked.
pub const FALLBACK_WAIT: Duration = Duration::from_secs(60);

/// A periodic background job.
pub trait Job: Send + 'static {
    /// For the logs.
    const NAME: &'static str;

    /// Wait between two runs; read before every wait, so a changed setting applies from the
    /// next one.
    fn interval(&self) -> Duration;

    /// Wait before the first run.
    fn first_delay(&self) -> Duration {
        self.interval()
    }

    /// One run; logs its own failures. Must be cancel-safe.
    fn run(&mut self) -> impl Future<Output = ()> + Send;

    /// After the loop stopped and the servers drained, before the database closes.
    fn finish(&mut self) -> impl Future<Output = ()> + Send {
        std::future::ready(())
    }
}

type Finishing = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The running jobs.
pub struct Scheduler {
    token: CancellationToken,
    finishing: Vec<Finishing>,
}

impl Scheduler {
    /// Jobs added later run until `token` is cancelled.
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            finishing: Vec::new(),
        }
    }

    /// Starts `job`'s loop.
    pub fn spawn<J: Job>(&mut self, job: J) {
        let handle: JoinHandle<J> = tokio::spawn(run_loop(job, self.token.clone()));
        self.finishing.push(Box::pin(async move {
            match handle.await {
                Ok(mut job) => {
                    if let Err(panic) = CatchUnwind(Box::pin(job.finish())).await {
                        tracing::error!(
                            job = J::NAME,
                            panic = panic_message(&panic),
                            "job finish panicked"
                        );
                    }
                }
                Err(err) => tracing::error!(job = J::NAME, error = %err, "job task failed"),
            }
        }));
    }

    /// Waits for every loop to stop (the token must be cancelled) and runs each job's
    /// [`Job::finish`], within `timeout`; `false` if that ran out.
    pub async fn finish(self, timeout: Duration) -> bool {
        let all = async {
            for finishing in self.finishing {
                finishing.await;
            }
        };
        let done = tokio::time::timeout(timeout, all).await.is_ok();
        if !done {
            tracing::warn!(?timeout, "background jobs did not finish in time");
        }
        done
    }
}

async fn run_loop<J: Job>(mut job: J, token: CancellationToken) -> J {
    let mut wait = wait_of(&job, J::first_delay);
    loop {
        tokio::select! {
            // A run due after the cancellation is left to `finish`.
            biased;
            () = token.cancelled() => return job,
            () = tokio::time::sleep(wait) => {}
        }
        tokio::select! {
            biased;
            () = token.cancelled() => return job,
            outcome = CatchUnwind(Box::pin(job.run())) => if let Err(panic) = outcome {
                tracing::error!(job = J::NAME, panic = panic_message(&panic), "job run panicked; running again at the next tick");
            },
        }
        wait = wait_of(&job, J::interval);
    }
}

/// `read(job)`, or [`FALLBACK_WAIT`] if it panicked.
fn wait_of<J: Job>(job: &J, read: fn(&J) -> Duration) -> Duration {
    std::panic::catch_unwind(AssertUnwindSafe(|| read(job))).unwrap_or_else(|panic| {
        tracing::error!(job = J::NAME, panic = panic_message(&panic), wait = ?FALLBACK_WAIT, "job interval panicked");
        FALLBACK_WAIT
    })
}

type Panic = Box<dyn Any + Send>;

/// The message a panic was raised with, when it has one.
fn panic_message(panic: &Panic) -> &str {
    if let Some(text) = panic.downcast_ref::<&'static str>() {
        text
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text
    } else {
        "<non-string panic payload>"
    }
}

/// `F`, with a panic while polling it turned into `Err` (it is not polled again).
struct CatchUnwind<F>(Pin<Box<F>>);

impl<F: Future> Future for CatchUnwind<F> {
    type Output = Result<F::Output, Panic>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = self.get_mut().0.as_mut();
        match std::panic::catch_unwind(AssertUnwindSafe(|| inner.poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Err(panic) => Poll::Ready(Err(panic)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const TICK: Duration = Duration::from_millis(5);
    const PATIENCE: Duration = Duration::from_secs(5);

    /// Counts its runs and finishes; panics on the runs `panic_on` says.
    struct Probe {
        runs: Arc<AtomicUsize>,
        finished: Arc<AtomicUsize>,
        panic_on: fn(usize) -> bool,
        first: Duration,
        every: Duration,
    }

    impl Probe {
        fn new() -> (Self, Arc<AtomicUsize>, Arc<AtomicUsize>) {
            let runs = Arc::new(AtomicUsize::new(0));
            let finished = Arc::new(AtomicUsize::new(0));
            let probe = Self {
                runs: Arc::clone(&runs),
                finished: Arc::clone(&finished),
                panic_on: |_| false,
                first: TICK,
                every: TICK,
            };
            (probe, runs, finished)
        }
    }

    impl Job for Probe {
        const NAME: &'static str = "probe";

        fn interval(&self) -> Duration {
            self.every
        }

        fn first_delay(&self) -> Duration {
            self.first
        }

        async fn run(&mut self) {
            let n = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
            if (self.panic_on)(n) {
                panic!("run {n} failed");
            }
        }

        async fn finish(&mut self) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn until(what: impl Fn() -> bool) {
        let waited = tokio::time::timeout(PATIENCE, async {
            while !what() {
                tokio::time::sleep(TICK).await;
            }
        })
        .await;
        assert!(waited.is_ok(), "condition never met");
    }

    #[tokio::test]
    async fn a_job_runs_every_interval_until_cancelled_then_finishes_once() {
        // Arrange
        let token = CancellationToken::new();
        let mut scheduler = Scheduler::new(token.clone());
        let (probe, runs, finished) = Probe::new();

        // Act
        scheduler.spawn(probe);
        until(|| runs.load(Ordering::SeqCst) >= 3).await;
        token.cancel();
        let done = scheduler.finish(PATIENCE).await;
        let after_finish = runs.load(Ordering::SeqCst);
        tokio::time::sleep(TICK * 4).await;

        // Assert
        assert!(done);
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(
            runs.load(Ordering::SeqCst),
            after_finish,
            "no run after stop"
        );
    }

    #[tokio::test]
    async fn a_panicking_run_neither_stops_its_job_nor_the_others() {
        // Arrange
        let token = CancellationToken::new();
        let mut scheduler = Scheduler::new(token.clone());
        let (mut flaky, flaky_runs, flaky_finished) = Probe::new();
        flaky.panic_on = |n| n % 2 == 1;
        let (steady, steady_runs, _) = Probe::new();

        // Act
        scheduler.spawn(flaky);
        scheduler.spawn(steady);
        until(|| flaky_runs.load(Ordering::SeqCst) >= 4).await;
        until(|| steady_runs.load(Ordering::SeqCst) >= 4).await;
        token.cancel();
        let done = scheduler.finish(PATIENCE).await;

        // Assert
        assert!(done);
        assert_eq!(flaky_finished.load(Ordering::SeqCst), 1, "still finished");
    }

    #[tokio::test]
    async fn the_first_run_waits_for_its_own_delay() {
        // Arrange
        let token = CancellationToken::new();
        let mut scheduler = Scheduler::new(token.clone());
        let (mut slow_start, runs, _) = Probe::new();
        slow_start.first = Duration::from_secs(3600);

        // Act
        scheduler.spawn(slow_start);
        tokio::time::sleep(TICK * 10).await;
        token.cancel();
        scheduler.finish(PATIENCE).await;

        // Assert
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }

    /// A run that never ends unless dropped.
    struct Stuck {
        started: Arc<AtomicUsize>,
    }

    impl Job for Stuck {
        const NAME: &'static str = "stuck";

        fn interval(&self) -> Duration {
            TICK
        }

        async fn run(&mut self) {
            self.started.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<()>().await;
        }
    }

    #[tokio::test]
    async fn cancellation_interrupts_a_run_in_progress() {
        // Arrange
        let token = CancellationToken::new();
        let mut scheduler = Scheduler::new(token.clone());
        let started = Arc::new(AtomicUsize::new(0));
        scheduler.spawn(Stuck {
            started: Arc::clone(&started),
        });
        until(|| started.load(Ordering::SeqCst) == 1).await;

        // Act
        token.cancel();
        let done = scheduler.finish(PATIENCE).await;

        // Assert
        assert!(done);
    }

    #[tokio::test]
    async fn finish_gives_up_after_its_timeout() {
        // Arrange: never cancelled, so the loop never returns.
        let token = CancellationToken::new();
        let mut scheduler = Scheduler::new(token.clone());
        let (probe, _, finished) = Probe::new();
        scheduler.spawn(probe);

        // Act
        let done = scheduler.finish(TICK * 4).await;

        // Assert
        assert!(!done);
        assert_eq!(finished.load(Ordering::SeqCst), 0);
        token.cancel();
    }

    /// Its interval panics after the first run; its finish counts.
    struct BadInterval {
        runs: Arc<AtomicUsize>,
        finished: Arc<AtomicUsize>,
    }

    impl Job for BadInterval {
        const NAME: &'static str = "bad_interval";

        fn interval(&self) -> Duration {
            panic!("setting unreadable");
        }

        fn first_delay(&self) -> Duration {
            TICK
        }

        async fn run(&mut self) {
            self.runs.fetch_add(1, Ordering::SeqCst);
        }

        async fn finish(&mut self) {
            self.finished.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn a_panicking_interval_keeps_the_job_and_its_finish() {
        // Arrange
        let token = CancellationToken::new();
        let mut scheduler = Scheduler::new(token.clone());
        let runs = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicUsize::new(0));
        scheduler.spawn(BadInterval {
            runs: Arc::clone(&runs),
            finished: Arc::clone(&finished),
        });

        // Act: the first run, then a panicking interval (the loop waits the fallback).
        until(|| runs.load(Ordering::SeqCst) == 1).await;
        tokio::time::sleep(TICK * 4).await;
        token.cancel();
        let done = scheduler.finish(PATIENCE).await;

        // Assert: the task survived, so the final work still ran.
        assert!(done);
        assert_eq!(finished.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn panic_messages_are_read_from_str_and_string_payloads() {
        let text: Panic = Box::new("static text");
        let owned: Panic = Box::new(String::from("formatted 7"));
        let other: Panic = Box::new(7_u8);

        assert_eq!(panic_message(&text), "static text");
        assert_eq!(panic_message(&owned), "formatted 7");
        assert_eq!(panic_message(&other), "<non-string panic payload>");
    }
}
