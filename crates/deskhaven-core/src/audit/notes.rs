//! [`PendingNotes`]: controller notes waiting for their session to appear.
//!
//! The controller posts its note with the session id only (`src/ui_session_interface.rs:
//! 2104-2107`); `connection_sessions.conn_id` is `NOT NULL`, so a note cannot create a row.
//! The session id reaches a row with the controlled device's authorization record, which can
//! be held up behind the device's retries of its opening record (sequential, up to 120 s
//! each, `src/server/connection.rs:1331-1336`), while the controller's note goes out at once.
//! Such a note waits here until a record binds the session, at most [`NOTE_TTL`].
//!
//! - **Bounded**: at most [`MAX_PENDING_NOTES`] notes; expired ones are dropped when full,
//!   and a note that still does not fit is refused (the caller logs it).
//! - **Monotonic**: deadlines run on [`Clock::elapsed_ms`](crate::clock::Clock::elapsed_ms).
//! - **In memory only**: a restart loses waiting notes (the note is a user annotation, the
//!   connection records themselves are stored).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::SessionId;
use crate::clock::SharedClock;

/// How long a note waits for its session: the device's opening record may take two retries
/// within the client's 120 s deadline, its authorization record as long again.
pub const NOTE_TTL: Duration = Duration::from_secs(5 * 60);
/// Notes waiting at most (a few hundred KiB at the longest notes).
pub const MAX_PENDING_NOTES: usize = 1_000;

type Key = (String, SessionId);

/// Waiting notes by (device RustDesk id, session id): the note and when it arrived
/// (monotonic ms).
pub(super) struct PendingNotes {
    clock: SharedClock,
    notes: Mutex<HashMap<Key, (String, i64)>>,
}

impl PendingNotes {
    pub fn new(clock: SharedClock) -> Self {
        Self {
            clock,
            notes: Mutex::default(),
        }
    }

    /// Keeps `note` for the session (replacing an older one); `false` if it does not fit.
    pub fn hold(&self, device: &str, session: &SessionId, note: String) -> bool {
        let now = self.clock.elapsed_ms();
        let key = (device.to_owned(), session.clone());
        let mut notes = self.lock();
        if notes.len() >= MAX_PENDING_NOTES && !notes.contains_key(&key) {
            notes.retain(|_, (_, at)| !expired(*at, now));
            if notes.len() >= MAX_PENDING_NOTES {
                return false;
            }
        }
        notes.insert(key, (note, now));
        true
    }

    /// Removes and returns the session's note, unless it expired.
    pub fn take(&self, device: &str, session: &SessionId) -> Option<String> {
        let now = self.clock.elapsed_ms();
        let key = (device.to_owned(), session.clone());
        let (note, at) = self.lock().remove(&key)?;
        (!expired(at, now)).then_some(note)
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Key, (String, i64)>> {
        // Map operations cannot leave it inconsistent.
        self.notes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn expired(at: i64, now: i64) -> bool {
    let ttl = i64::try_from(NOTE_TTL.as_millis()).unwrap_or(i64::MAX);
    now.saturating_sub(at) >= ttl
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::clock::ManualClock;

    fn session(id: u64) -> SessionId {
        SessionId::from_json(&json!(id)).unwrap()
    }

    #[test]
    fn a_note_is_taken_once_and_a_newer_one_replaces_it() {
        let notes = PendingNotes::new(Arc::new(ManualClock::at(1_000)));

        assert!(notes.hold("a", &session(1), "old".to_owned()));
        assert!(notes.hold("a", &session(1), "new".to_owned()));

        assert_eq!(notes.take("b", &session(1)), None, "per device");
        assert_eq!(notes.take("a", &session(1)).as_deref(), Some("new"));
        assert_eq!(notes.take("a", &session(1)), None);
    }

    #[test]
    fn a_note_expires_on_the_monotonic_clock() {
        let clock = Arc::new(ManualClock::at(1_000));
        let notes = PendingNotes::new(clock.clone());
        notes.hold("a", &session(1), "x".to_owned());
        notes.hold("a", &session(2), "y".to_owned());

        clock.step_wall(-3_600_000);
        clock.advance(NOTE_TTL - Duration::from_millis(1));
        assert_eq!(notes.take("a", &session(1)).as_deref(), Some("x"));
        clock.advance(Duration::from_millis(1));
        assert_eq!(notes.take("a", &session(2)), None);
    }

    #[test]
    fn a_full_buffer_drops_expired_notes_then_refuses() {
        let clock = Arc::new(ManualClock::at(1_000));
        let notes = PendingNotes::new(clock.clone());
        for i in 0..MAX_PENDING_NOTES as u64 {
            assert!(notes.hold("a", &session(i), String::new()));
        }

        let refused = notes.hold("a", &session(u64::MAX), String::new());
        let replaced = notes.hold("a", &session(0), "again".to_owned());
        clock.advance(NOTE_TTL);
        let after_expiry = notes.hold("a", &session(u64::MAX), String::new());

        assert!(!refused);
        assert!(replaced, "a session already waiting takes no new slot");
        assert!(after_expiry);
        assert_eq!(notes.lock().len(), 1);
    }
}
