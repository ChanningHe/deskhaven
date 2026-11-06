//! Accepting through resource exhaustion (deskhaven code, not vendored; M4-05 review LOW-1).
//!
//! Upstream rebound a listener on every accept error. When the process is out of file
//! descriptors (EMFILE / ENFILE) that drops every connection waiting in the backlog, and the
//! new socket may fail to be created at all, which ends the server. Such errors, and the ones
//! about a single connection that died in the backlog, are transient: the listener pauses
//! for [`BACKOFF`] (it would otherwise spin, the backlog entry stays) and accepts again. Any
//! other error is returned, and the caller rebinds as before.

use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use hbb_common::log;
use hbb_common::tokio::net::{TcpListener, TcpStream};
use hbb_common::tokio::time::{Instant as TokioInstant, sleep_until};

use crate::log_limit::LogLimit;

/// Pause of a listener after a transient accept error.
pub(crate) const BACKOFF: Duration = Duration::from_millis(250);
/// `ENFILE` / `EMFILE`: the same numbers on Linux, macOS and the BSDs.
const ENFILE: i32 = 23;
const EMFILE: i32 = 24;

static TRANSIENT_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));

/// `listener.accept()`, waiting out transient errors. `pause` keeps the end of the current
/// pause across calls, so the future can be dropped (by `select!`) and created again without
/// cutting the pause short or extending it.
pub(crate) async fn accept(
    listener: &TcpListener,
    pause: &mut Option<TokioInstant>,
) -> io::Result<(TcpStream, SocketAddr)> {
    loop {
        if let Some(until) = *pause {
            sleep_until(until).await;
            *pause = None;
        }
        match listener.accept().await {
            Err(err) if is_transient(&err) => {
                if let Some(count) = TRANSIENT_LOG.hit(Instant::now()) {
                    log::warn!("{count} transient accept error(s), pausing the listener: {err}");
                }
                *pause = Some(TokioInstant::now() + BACKOFF);
            }
            other => return other,
        }
    }
}

fn is_transient(err: &io::Error) -> bool {
    use io::ErrorKind::*;
    matches!(err.raw_os_error(), Some(ENFILE | EMFILE))
        || matches!(
            err.kind(),
            ConnectionAborted | ConnectionReset | Interrupted | OutOfMemory
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_exhaustion_and_dead_backlog_entries_are_transient() {
        for code in [ENFILE, EMFILE] {
            assert!(is_transient(&io::Error::from_raw_os_error(code)));
        }
        assert!(is_transient(&io::ErrorKind::ConnectionAborted.into()));
        assert!(!is_transient(&io::ErrorKind::InvalidInput.into()));
    }
}
