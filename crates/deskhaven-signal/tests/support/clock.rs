//! Real I/O under tokio's paused clock.
//!
//! A `start_paused` runtime jumps the clock to the next timer whenever it finds no ready task
//! and no I/O event at that instant. Loopback delivery is not instantaneous (under CPU load the
//! kernel defers it to ksoftirqd), so an accept, a frame or a close that is merely *in flight*
//! looks like idleness: the clock leaps over the server's timers and the client's own `STEP` /
//! `NEVER` deadlines before the event is seen ("closed after 32.98 s instead of 30 s", "websocket
//! connect + handshake within STEP", "a first frame").
//!
//! The cure is to never let the clock run while the test waits for real I/O:
//! - [`io`] holds a blocking-pool task for the duration of the wait; tokio does not auto-advance
//!   a paused clock while one runs, so virtual timers are inert and the wait ends on the event
//!   itself, bounded by [`IO_DEADLINE`] of *real* time. Without a paused clock it changes
//!   nothing (the awaited future keeps its own real-time timeouts).
//! - Virtual time is only moved by `tokio::time::advance`, from a state where nothing is in
//!   flight ([`super::TestServer::settle`]).

use hbb_common::tokio::{self, time::Duration};
use std::{future::Future, sync::mpsc};

/// Longest real time any single wait for an I/O event may take, however loaded the machine.
pub const IO_DEADLINE: Duration = Duration::from_secs(30);

/// Awaits `fut` (a wait for real I/O) with a paused clock held still; panics if it has not
/// completed after [`IO_DEADLINE`] of real time.
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
