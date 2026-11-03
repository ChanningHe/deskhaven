//! [`PendingNotes`]: controller notes waiting for their session to appear.
//!
//! The controller posts its note with the session id only (`src/ui_session_interface.rs:
//! 2104-2107`); `connection_sessions.conn_id` is `NOT NULL`, so a note cannot create a row.
//! The session id reaches a row with the controlled device's authorization record, which can
//! be held up behind the device's retries of its opening record (sequential, up to 120 s
//! each, `src/server/connection.rs:1331-1336`), while the controller's note goes out at once.
//! Such a note waits here until a record binds the session, at most [`NOTE_TTL`].
//!
//! The note carries no uuid, so anyone who knows a registered id can post one. Hence:
//! - **Bounded three ways**, never evicting: at most [`MAX_NOTES_PER_SOURCE`] waiting notes
//!   per client network (the rate limiter's key: IPv4 /32, IPv6 /64, one shared key for
//!   proxy-fallback clients), [`MAX_NOTES_PER_SOURCE_DEVICE`] per client network and device,
//!   [`MAX_PENDING_NOTES`] in all. A note that does not fit is refused; waiting notes stay
//!   until taken or expired.
//! - **First wins**: a session's waiting note is not replaced (06 §7).
//! - **Monotonic**: deadlines run on [`Clock::elapsed_ms`](crate::clock::Clock::elapsed_ms).
//! - **In memory only**: a restart loses waiting notes (the note is a user annotation, the
//!   connection records themselves are stored).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::{PAIRING_WINDOW, SessionId};
use crate::auth::limiter::{ClientAddr, ip_key};
use crate::clock::SharedClock;

/// How long a note waits for its session: as long as a connection's records pair.
pub const NOTE_TTL: Duration = PAIRING_WINDOW;
/// Notes waiting at most (a few MiB at the longest notes).
pub const MAX_PENDING_NOTES: usize = 1_000;
/// Waiting notes per client network at most.
pub const MAX_NOTES_PER_SOURCE: usize = 16;
/// Waiting notes per (client network, device) at most: one source cannot take a device's
/// slots from its other controllers.
pub const MAX_NOTES_PER_SOURCE_DEVICE: usize = 8;
/// At most one warning per interval about a full buffer.
const FULL_WARN_INTERVAL_MS: i64 = 60_000;

/// The client network a note came from; `None`: a proxy-fallback client.
type Source = Option<IpAddr>;
type Key = (String, SessionId);

struct Entry {
    note: String,
    /// Arrival, monotonic ms.
    at: i64,
    source: Source,
}

/// What [`PendingNotes::hold`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hold {
    Held,
    /// A note of the session was already waiting; it stays.
    AlreadyWaiting,
    SourceFull,
    DeviceFull,
    Full,
}

pub(super) struct PendingNotes {
    clock: SharedClock,
    notes: Mutex<HashMap<Key, Entry>>,
    /// Monotonic ms + 1 of the last "full" warning; 0 = never.
    warned: AtomicI64,
}

impl PendingNotes {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            clock,
            notes: Mutex::default(),
            warned: AtomicI64::new(0),
        }
    }

    /// Keeps `note` for the session if it fits (see the module documentation).
    pub fn hold(&self, device: &str, session: &SessionId, note: String, from: ClientAddr) -> Hold {
        let now = self.clock.elapsed_ms();
        let source = (!from.is_proxy_fallback).then(|| ip_key(from.addr));
        let key = (device.to_owned(), session.clone());
        let mut notes = self.lock();
        notes.retain(|_, entry| !expired(entry.at, now));
        if notes.contains_key(&key) {
            return Hold::AlreadyWaiting;
        }
        let hold = refusal(&notes, device, source).unwrap_or(Hold::Held);
        if hold == Hold::Held {
            notes.insert(
                key,
                Entry {
                    note,
                    at: now,
                    source,
                },
            );
        }
        hold
    }

    /// Removes and returns the session's note, unless it expired.
    pub fn take(&self, device: &str, session: &SessionId) -> Option<String> {
        let now = self.clock.elapsed_ms();
        let key = (device.to_owned(), session.clone());
        let entry = self.lock().remove(&key)?;
        (!expired(entry.at, now)).then_some(entry.note)
    }

    /// `true` at most once per minute: whether to warn that the buffer is full.
    pub fn warn_due(&self) -> bool {
        let now = self.clock.elapsed_ms();
        let last = self.warned.load(Ordering::Relaxed);
        let due = last == 0 || now.saturating_sub(last - 1) >= FULL_WARN_INTERVAL_MS;
        // Of concurrent callers, the one that moves the mark warns.
        let (set, order) = (now.saturating_add(1), Ordering::Relaxed);
        due && self
            .warned
            .compare_exchange(last, set, order, order)
            .is_ok()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Key, Entry>> {
        // Map operations cannot leave it inconsistent.
        self.notes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Why a new note of `device` from `source` does not fit, if it does not.
fn refusal(notes: &HashMap<Key, Entry>, device: &str, source: Source) -> Option<Hold> {
    if notes.len() >= MAX_PENDING_NOTES {
        return Some(Hold::Full);
    }
    let from_source = notes.values().filter(|e| e.source == source).count();
    if from_source >= MAX_NOTES_PER_SOURCE {
        return Some(Hold::SourceFull);
    }
    let from_source_for_device = notes
        .iter()
        .filter(|((d, _), e)| d == device && e.source == source)
        .count();
    (from_source_for_device >= MAX_NOTES_PER_SOURCE_DEVICE).then_some(Hold::DeviceFull)
}

fn expired(at: i64, now: i64) -> bool {
    let ttl = i64::try_from(NOTE_TTL.as_millis()).unwrap_or(i64::MAX);
    now.saturating_sub(at) >= ttl
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::clock::ManualClock;

    fn session(id: u64) -> SessionId {
        SessionId::from_json(&json!(id)).unwrap()
    }

    fn from(last: u8) -> ClientAddr {
        ClientAddr {
            addr: IpAddr::V4(Ipv4Addr::new(192, 0, 2, last)),
            is_proxy_fallback: false,
        }
    }

    #[test]
    fn a_note_is_taken_once_and_the_first_one_stays() {
        let notes = PendingNotes::new(Arc::new(ManualClock::at(1_000)));

        assert_eq!(
            notes.hold("a", &session(1), "old".to_owned(), from(1)),
            Hold::Held
        );
        let again = notes.hold("a", &session(1), "new".to_owned(), from(2));
        assert_eq!(again, Hold::AlreadyWaiting);

        assert_eq!(notes.take("b", &session(1)), None, "per device");
        assert_eq!(notes.take("a", &session(1)).as_deref(), Some("old"));
        assert_eq!(notes.take("a", &session(1)), None);
    }

    #[test]
    fn a_note_expires_on_the_monotonic_clock() {
        let clock = Arc::new(ManualClock::at(1_000));
        let notes = PendingNotes::new(clock.clone());
        notes.hold("a", &session(1), "x".to_owned(), from(1));
        notes.hold("a", &session(2), "y".to_owned(), from(1));

        clock.step_wall(-3_600_000);
        clock.advance(NOTE_TTL - Duration::from_millis(1));
        assert_eq!(notes.take("a", &session(1)).as_deref(), Some("x"));
        clock.advance(Duration::from_millis(1));
        assert_eq!(notes.take("a", &session(2)), None);
    }

    #[test]
    fn caps_per_source_per_device_and_overall_never_evict() {
        let clock = Arc::new(ManualClock::at(1_000));
        let notes = PendingNotes::new(clock.clone());
        let hold = |device: &str, id: u64, last: u8| {
            notes.hold(device, &session(id), String::new(), from(last))
        };
        for i in 0..MAX_NOTES_PER_SOURCE as u64 {
            assert_eq!(hold(&format!("d{i}"), i, 1), Hold::Held);
        }
        assert_eq!(hold("x", 0, 1), Hold::SourceFull);
        for i in 0..MAX_NOTES_PER_SOURCE_DEVICE as u64 {
            assert_eq!(hold("x", i, 2), Hold::Held);
        }
        assert_eq!(hold("x", 99, 2), Hold::DeviceFull);
        assert_eq!(
            hold("x", 99, 3),
            Hold::Held,
            "another source has its own slots"
        );
        let mut held = notes.lock().len() as u64;
        while held < MAX_PENDING_NOTES as u64 {
            let last = u8::try_from(100 + held / MAX_NOTES_PER_SOURCE as u64).unwrap_or(255);
            let device = format!("e{held}");
            assert_eq!(hold(&device, held, last), Hold::Held, "{held}");
            held += 1;
        }
        assert_eq!(hold("y", 0, 0), Hold::Full);

        clock.advance(NOTE_TTL);
        assert_eq!(hold("y", 0, 0), Hold::Held, "expired notes make room");
    }

    #[test]
    fn the_full_warning_is_rate_limited() {
        let clock = Arc::new(ManualClock::at(1_000));
        let notes = PendingNotes::new(clock.clone());

        assert!(notes.warn_due());
        assert!(!notes.warn_due());
        clock.advance(Duration::from_secs(60));
        assert!(notes.warn_due());
    }
}
