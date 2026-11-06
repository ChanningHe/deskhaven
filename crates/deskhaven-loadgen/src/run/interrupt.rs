//! SIGINT / SIGTERM, latched for the whole run: one watcher task trips it once, and every
//! later [`Interrupt::guard`] or [`Interrupt::check`] fails, so a signal during a server stop
//! still cancels what would come next (the idle restart).

use std::future::Future;
use std::sync::{Arc, OnceLock};

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::LoadgenError;

#[derive(Clone, Default)]
pub(super) struct Interrupt {
    token: CancellationToken,
    by: Arc<OnceLock<&'static str>>,
}

impl Interrupt {
    /// Starts watching the signals; the watcher is aborted when the returned set is dropped.
    pub(super) fn watch() -> (Self, JoinSet<()>) {
        let interrupt = Self::default();
        let mut watcher = JoinSet::new();
        let tripped = interrupt.clone();
        watcher.spawn(async move { tripped.trip(signal().await) });
        (interrupt, watcher)
    }

    fn trip(&self, by: &'static str) {
        // The first signal names the interruption.
        let _ = self.by.set(by);
        self.token.cancel();
    }

    fn error(&self) -> LoadgenError {
        LoadgenError::Interrupted(self.by.get().copied().unwrap_or("a signal"))
    }

    /// `Err` once a signal came.
    pub(super) fn check(&self) -> Result<(), LoadgenError> {
        match self.token.is_cancelled() {
            true => Err(self.error()),
            false => Ok(()),
        }
    }

    /// `work`, unless a signal comes (or came) first; `work` is then dropped, which aborts
    /// the device and sampler tasks, and the caller stops the server.
    pub(super) async fn guard<T>(
        &self,
        work: impl Future<Output = Result<T, LoadgenError>>,
    ) -> Result<T, LoadgenError> {
        tokio::select! {
            biased;
            () = self.token.cancelled() => Err(self.error()),
            result = work => result,
        }
    }
}

#[cfg(unix)]
async fn signal() -> &'static str {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut term), Ok(mut int)) = (
        signal(SignalKind::terminate()),
        signal(SignalKind::interrupt()),
    ) else {
        tracing::warn!("cannot watch SIGINT / SIGTERM; interrupting will orphan the server");
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => "SIGTERM",
        _ = int.recv() => "SIGINT",
    }
}

#[cfg(not(unix))]
async fn signal() -> &'static str {
    match tokio::signal::ctrl_c().await {
        Ok(()) => "Ctrl-C",
        Err(_) => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_signal_cancels_the_running_work_and_everything_after_it() {
        // Arrange
        let interrupt = Interrupt::default();
        assert!(interrupt.check().is_ok());

        // Act: tripped while work is pending.
        let tripper = interrupt.clone();
        let running = interrupt.guard(async {
            tripper.trip("SIGINT");
            std::future::pending::<Result<(), LoadgenError>>().await
        });
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), running)
            .await
            .expect("guard must return once tripped");

        // Assert: that work and any later step fail, without another signal.
        assert!(matches!(first, Err(LoadgenError::Interrupted("SIGINT"))));
        assert!(matches!(
            interrupt.check(),
            Err(LoadgenError::Interrupted("SIGINT"))
        ));
        let later = interrupt.guard(async { Ok(()) }).await;
        assert!(matches!(later, Err(LoadgenError::Interrupted("SIGINT"))));
    }

    #[tokio::test]
    async fn without_a_signal_work_runs_to_its_end() {
        let interrupt = Interrupt::default();

        assert_eq!(interrupt.guard(async { Ok(7) }).await.ok(), Some(7));
        assert!(interrupt.check().is_ok());
    }
}
