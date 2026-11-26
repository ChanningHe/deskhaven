//! A `start_paused` runtime jumps the clock to the next timer whenever nothing is ready, and
//! loopback I/O merely in flight looks like idleness. So tests never let the clock run while
//! waiting for real I/O: [`io`] holds a blocking-pool task meanwhile (tokio does not
//! auto-advance while one runs), and virtual time moves only by `advance` from a settled server.

use hbb_common::tokio::{self, time::Duration};
use std::{future::Future, sync::mpsc};

pub const IO_DEADLINE: Duration = Duration::from_secs(30);

pub async fn io<F: Future>(what: &str, fut: F) -> F::Output {
    let (release, held) = mpsc::channel::<()>();
    let mut hold = tokio::task::spawn_blocking(move || held.recv_timeout(IO_DEADLINE));
    let finished = tokio::select! {
        out = fut => Ok(out),
        res = &mut hold => Err(res),
    };
    match finished {
        Ok(out) => {
            drop(release);
            // The clock may move again once the hold is gone.
            let _ = hold.await;
            out
        }
        Err(_) => panic!("{what}: no progress within {IO_DEADLINE:?} of real time"),
    }
}
