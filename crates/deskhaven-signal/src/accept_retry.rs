//! Accepting through resource exhaustion (deskhaven code, not vendored; M4-05 review LOW-1).
//!
//! Upstream rebound a listener on every accept error. When the process is out of file
//! descriptors (EMFILE / ENFILE) that drops every connection waiting in the backlog, and the
//! new socket may fail to be created at all, which ends the server. On such errors the
//! listener pauses for [`BACKOFF`] (it would otherwise spin, the backlog entry stays) and
//! accepts again. Errors of a single connection (reset or aborted in the backlog, network
//! errors Linux passes on) consumed that entry: the next one is accepted at once. Any other
//! error is returned, and the caller rebinds as before.

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
        let err = match listener.accept().await {
            Err(err) => err,
            ok => return ok,
        };
        match classify(&err) {
            Action::Fail => return Err(err),
            Action::Retry => log::debug!("accept failed for one connection: {err}"),
            Action::Pause => {
                if let Some(count) = TRANSIENT_LOG.hit(Instant::now()) {
                    log::warn!("{count} transient accept error(s), pausing the listener: {err}");
                }
                *pause = Some(TokioInstant::now() + BACKOFF);
            }
        }
    }
}

/// What to do after a failed accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// The error concerned one backlog entry, which it consumed: accept the next one now.
    Retry,
    /// The process or system is out of a resource: wait [`BACKOFF`], the entry stays queued.
    Pause,
    /// Anything else: the caller rebinds the listener.
    Fail,
}

fn classify(err: &io::Error) -> Action {
    use io::ErrorKind::*;
    if matches!(err.raw_os_error(), Some(ENFILE | EMFILE)) || err.kind() == OutOfMemory {
        return Action::Pause;
    }
    let one_connection = matches!(
        err.kind(),
        ConnectionAborted
            | ConnectionReset
            | Interrupted
            | NetworkUnreachable
            | HostUnreachable
            | TimedOut
    ) || is_linux_network_error(err);
    if one_connection {
        Action::Retry
    } else {
        Action::Fail
    }
}

/// accept(2) on Linux passes on pending network errors of the new socket as its own (man
/// page, "Error handling"): EPROTO, ENONET, EHOSTDOWN, ENOPROTOOPT, EOPNOTSUPP. The
/// unreachable / timed-out ones are matched by kind above.
#[cfg(target_os = "linux")]
fn is_linux_network_error(err: &io::Error) -> bool {
    const LINUX_NETWORK_ERRORS: [i32; 5] = [71, 64, 112, 92, 95];
    err.raw_os_error()
        .is_some_and(|code| LINUX_NETWORK_ERRORS.contains(&code))
}

#[cfg(not(target_os = "linux"))]
fn is_linux_network_error(_: &io::Error) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_exhaustion_pauses_the_listener() {
        for code in [ENFILE, EMFILE] {
            assert_eq!(classify(&io::Error::from_raw_os_error(code)), Action::Pause);
        }
        assert_eq!(classify(&io::ErrorKind::OutOfMemory.into()), Action::Pause);
    }

    /// Review LOW-1: these concern one backlog entry, which the failed accept consumed.
    #[test]
    fn errors_of_one_connection_retry_at_once() {
        use io::ErrorKind::*;
        for kind in [
            ConnectionReset,
            ConnectionAborted,
            Interrupted,
            NetworkUnreachable,
            HostUnreachable,
            TimedOut,
        ] {
            assert_eq!(classify(&kind.into()), Action::Retry, "{kind:?}");
        }
        #[cfg(target_os = "linux")]
        for code in [71, 64, 112, 92, 95] {
            let err = io::Error::from_raw_os_error(code);
            assert_eq!(classify(&err), Action::Retry, "{err}");
        }
    }

    #[test]
    fn other_errors_fail_so_the_caller_rebinds() {
        assert_eq!(classify(&io::ErrorKind::InvalidInput.into()), Action::Fail);
    }
}
