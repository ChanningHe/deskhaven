//! Background tasks of one server instance: all of them stop when it stops.

use std::future::Future;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// Spawns tasks that end on `stop` and can all be awaited, so nothing outlives `run`.
#[derive(Clone)]
pub(crate) struct Tasks {
    tracker: TaskTracker,
    stop: CancellationToken,
}

impl Tasks {
    pub(crate) fn new(stop: CancellationToken) -> Self {
        Self {
            tracker: TaskTracker::new(),
            stop,
        }
    }

    /// Replacement for `tokio::spawn`: the task is dropped at its next await once stopped.
    pub(crate) fn spawn<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let stop = self.stop.clone();
        self.tracker.spawn(async move {
            stop.run_until_cancelled(task).await;
        });
    }

    /// Stops every task and waits until all of them have finished.
    pub(crate) async fn shutdown(&self) {
        self.stop.cancel();
        self.tracker.close();
        self.tracker.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio::{self, sync::oneshot};

    #[tokio::test]
    async fn shutdown_cancels_and_awaits_pending_tasks() {
        let tasks = Tasks::new(CancellationToken::new());
        let (tx, rx) = oneshot::channel::<()>();
        tasks.spawn(async move {
            let _keep = tx;
            std::future::pending::<()>().await;
        });

        tasks.shutdown().await;

        assert!(rx.await.is_err(), "task (and its sender) was dropped");
    }
}
