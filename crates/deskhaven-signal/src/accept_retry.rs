//! On EMFILE / ENFILE a listener pauses for [`BACKOFF`] instead of rebinding: a rebind drops
//! the whole backlog and may fail to create the socket at all. Errors of a single backlog
//! entry are retried at once; anything else makes the caller rebind.

use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use hbb_common::log;
use hbb_common::tokio::net::{TcpListener, TcpStream};
use hbb_common::tokio::time::{Instant as TokioInstant, sleep_until};

use crate::log_limit::LogLimit;

pub(crate) const BACKOFF: Duration = Duration::from_millis(250);
/// `ENFILE` / `EMFILE`: the same numbers on Linux, macOS and the BSDs.
const ENFILE: i32 = 23;
const EMFILE: i32 = 24;

static TRANSIENT_LOG: LogLimit = LogLimit::new(Duration::from_secs(60));

/// `pause` keeps the end of the current pause across calls, so the future can be dropped (by
/// `select!`) and recreated without changing the pause.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Retry,
    Pause,
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

/// Linux accept(2) passes on pending network errors of the new socket as its own.
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
