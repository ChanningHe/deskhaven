//! The open web client sessions: who owns each, its route's two candidate halves, and the
//! device's candidates received so far (the long poll reads them by index).
//!
//! Locks: the table's mutex is never held across an await. A session's sender is outside
//! any lock and its receiver behind its own async mutex, so a long poll waiting for the
//! device never holds up the browser's candidates.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::watch;

use super::port::{CandidateRejected, CandidateSink, CandidateSource};

/// How long a session is kept from its creation; its route (60 s from the punch) is gone
/// by then.
pub const SESSION_TTL: Duration = Duration::from_secs(60);
/// Sessions one user holds at once, punches in flight included.
pub const MAX_SESSIONS_PER_USER: usize = 4;
/// Device candidates a session keeps (the route passes at most 128).
const MAX_CANDIDATES: usize = 128;

/// One page of the device's candidates: those from index `after` on, and the index to ask
/// from next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidates {
    pub candidates: Vec<String>,
    pub next: usize,
}

/// The route is closed (no WebRTC session, expired, or the session was closed) and every
/// candidate it delivered was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ended;

pub(super) struct Session {
    pub(super) user_id: i64,
    pub(super) device_id: String,
    expires: Duration,
    sender: Box<dyn CandidateSink>,
    inbox: tokio::sync::Mutex<Inbox>,
    closed: watch::Sender<bool>,
}

struct Inbox {
    source: Box<dyn CandidateSource>,
    log: Vec<String>,
    ended: bool,
}

impl Inbox {
    fn page(&self, after: usize) -> Candidates {
        let from = after.min(self.log.len());
        Candidates {
            candidates: self.log[from..].to_vec(),
            next: self.log.len(),
        }
    }
}

impl Session {
    pub(super) fn send(&self, candidate: String) -> Result<(), CandidateRejected> {
        self.sender.send(candidate)
    }

    /// The device's candidates from index `after` on. Without any, waits up to `wait` for the
    /// next one (an empty page when none came). `Err` once the route is closed and nothing
    /// from `after` on is left, or when the session is closed meanwhile.
    pub(super) async fn poll(&self, after: usize, wait: Duration) -> Result<Candidates, Ended> {
        let mut closed = self.closed.subscribe();
        let read = async {
            let mut inbox = self.inbox.lock().await;
            if after >= inbox.log.len() && !inbox.ended && !wait.is_zero() {
                match inbox.source.next().await {
                    Some(candidate) if inbox.log.len() < MAX_CANDIDATES => {
                        inbox.log.push(candidate);
                    }
                    Some(_) => {}
                    None => inbox.ended = true,
                }
            }
            match inbox.ended && after >= inbox.log.len() {
                true => Err(Ended),
                false => Ok(inbox.page(after)),
            }
        };
        let empty = Candidates {
            candidates: Vec::new(),
            next: after,
        };
        tokio::select! {
            read = tokio::time::timeout(wait, read) => read.unwrap_or(Ok(empty)),
            _ = closed.wait_for(|closed| *closed) => Err(Ended),
        }
    }

    fn close(&self) {
        self.closed.send_replace(true);
    }
}

/// What makes a session, besides its owner and id.
pub(super) struct NewSession {
    pub(super) device_id: String,
    pub(super) sender: Box<dyn CandidateSink>,
    pub(super) receiver: Box<dyn CandidateSource>,
}

#[derive(Default)]
struct Slots {
    sessions: HashMap<String, Arc<Session>>,
    /// Punches in flight per user: they count towards [`MAX_SESSIONS_PER_USER`].
    pending: HashMap<i64, usize>,
}

impl Slots {
    /// Drops expired sessions (closing them) as of `now` (monotonic).
    fn sweep(&mut self, now: Duration) {
        self.sessions.retain(|_, session| {
            let alive = now < session.expires;
            if !alive {
                session.close();
            }
            alive
        });
    }
}

#[derive(Default)]
pub(super) struct Table(Mutex<Slots>);

/// A user's slot held for a punch in flight; released on drop.
pub(super) struct Reservation<'a> {
    table: &'a Table,
    user_id: i64,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let mut slots = self.table.lock();
        if let Entry::Occupied(mut pending) = slots.pending.entry(self.user_id) {
            *pending.get_mut() -= 1;
            if *pending.get() == 0 {
                pending.remove();
            }
        }
    }
}

impl Table {
    fn lock(&self) -> MutexGuard<'_, Slots> {
        // Map operations cannot leave the table inconsistent.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A slot for a new session of `user_id`; `None` when the user holds
    /// [`MAX_SESSIONS_PER_USER`] (sessions and punches in flight).
    pub(super) fn reserve(&self, user_id: i64, now: Duration) -> Option<Reservation<'_>> {
        let mut slots = self.lock();
        slots.sweep(now);
        let open = slots
            .sessions
            .values()
            .filter(|s| s.user_id == user_id)
            .count();
        let pending = slots.pending.entry(user_id).or_default();
        if open + *pending >= MAX_SESSIONS_PER_USER {
            return None;
        }
        *pending += 1;
        Some(Reservation {
            table: self,
            user_id,
        })
    }

    /// Stores the session of `reservation` under `id`, alive until `now + SESSION_TTL`.
    pub(super) fn insert(
        &self,
        reservation: Reservation<'_>,
        id: String,
        new: NewSession,
        now: Duration,
    ) {
        let (closed, _) = watch::channel(false);
        let session = Session {
            user_id: reservation.user_id,
            device_id: new.device_id,
            expires: now + SESSION_TTL,
            sender: new.sender,
            inbox: tokio::sync::Mutex::new(Inbox {
                source: new.receiver,
                log: Vec::new(),
                ended: false,
            }),
            closed,
        };
        self.lock().sessions.insert(id, Arc::new(session));
        drop(reservation);
    }

    /// Session `id` if `user_id` owns it and it has not expired.
    pub(super) fn get(&self, user_id: i64, id: &str, now: Duration) -> Option<Arc<Session>> {
        let mut slots = self.lock();
        slots.sweep(now);
        slots
            .sessions
            .get(id)
            .filter(|s| s.user_id == user_id)
            .cloned()
    }

    /// Removes and closes session `id` if `user_id` owns it; its route goes once the last
    /// request using it returns.
    pub(super) fn remove(&self, user_id: i64, id: &str, now: Duration) -> Option<Arc<Session>> {
        let mut slots = self.lock();
        slots.sweep(now);
        let owned = slots.sessions.get(id).is_some_and(|s| s.user_id == user_id);
        let session = owned.then(|| slots.sessions.remove(id)).flatten()?;
        session.close();
        Some(session)
    }
}

#[cfg(test)]
#[path = "table_tests.rs"]
mod tests;
