use std::future::Future;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

/// Nothing outlives `run`: every task ends on `stop` and can be awaited.
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

    /// Not dropped when stopped but handed the stop token; must end promptly once it fires.
    pub(crate) fn spawn_draining<T, F>(&self, task: T)
    where
        T: FnOnce(CancellationToken) -> F,
        F: Future<Output = ()> + Send + 'static,
    {
        self.tracker.spawn(task(self.stop.clone()));
    }

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

    #[tokio::test]
    async fn shutdown_lets_draining_tasks_finish_their_work() {
        let tasks = Tasks::new(CancellationToken::new());
        let (tx, rx) = oneshot::channel::<()>();
        tasks.spawn_draining(|stop| async move {
            stop.cancelled().await;
            tokio::task::yield_now().await;
            tx.send(()).ok();
        });

        tasks.shutdown().await;

        assert!(rx.await.is_ok(), "task ran to completion after the stop");
    }
}
