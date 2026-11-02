//! In-memory cache of validated sessions (06 §3.2: hash → session, TTL 60s, explicit
//! invalidation on revocation).
//!
//! A plain `HashMap` behind a `std::sync::Mutex`: lookups are synchronous (usable from the
//! hbbs ACL path without an executor), critical sections are a few map operations and never
//! span an `.await`. Bounded: when full, stale entries are purged first, then one arbitrary
//! entry is evicted. Rare whole-cache scans (invalidate by user, flush) are O(n), which is
//! fine at the expected size.
//!
//! Each entry stores the session's absolute expiry and last use, so expiry is enforced on a
//! hit without the database ([`CachedSession::check`]). Entries expire after the cache TTL,
//! which bounds how long a change made without an explicit invalidation (a lost event, a
//! role change) can go unnoticed.
//!
//! Fill race: a request that read a session from the database must not re-insert it after
//! a concurrent revocation invalidated it. Every invalidation bumps a generation counter;
//! a fill passes the generation it saw *before* its database read and is dropped if any
//! invalidation happened since.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::{SessionError, SessionPrincipal, TokenHash};

/// How long an entry may serve without going back to the database.
pub const CACHE_TTL: Duration = Duration::from_secs(60);
/// Default bound on cached sessions.
pub const DEFAULT_CAPACITY: usize = 50_000;

/// A validated session as cached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedSession {
    pub principal: SessionPrincipal,
    /// Absolute expiry, unix ms.
    pub expires_at_ms: i64,
    /// Last recorded use, unix ms (the idle timeout counts from here).
    pub last_used_at_ms: i64,
}

impl CachedSession {
    /// [`SessionError::Expired`] past the absolute expiry or after `idle_ms` without use.
    pub fn check(&self, now_ms: i64, idle_ms: i64) -> Result<(), SessionError> {
        let idle_deadline = self.last_used_at_ms.saturating_add(idle_ms);
        if now_ms >= self.expires_at_ms || now_ms >= idle_deadline {
            return Err(SessionError::Expired);
        }
        Ok(())
    }
}

/// Opaque snapshot of the invalidation counter, taken before a database read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheGeneration(u64);

/// Shared handle; clones see the same entries.
#[derive(Clone)]
pub struct SessionCache {
    state: Arc<Mutex<State>>,
    ttl_ms: i64,
    capacity: usize,
}

#[derive(Default)]
struct State {
    entries: HashMap<TokenHash, Entry>,
    generation: u64,
}

struct Entry {
    session: CachedSession,
    cached_at_ms: i64,
}

impl Default for SessionCache {
    fn default() -> Self {
        Self::new(CACHE_TTL, DEFAULT_CAPACITY)
    }
}

impl SessionCache {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            state: Arc::default(),
            ttl_ms: i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX),
            capacity: capacity.max(1),
        }
    }

    /// The cached session, unless absent or older than the TTL (then dropped). Does not
    /// check session expiry: see [`CachedSession::check`].
    pub fn get(&self, hash: &TokenHash, now_ms: i64) -> Option<CachedSession> {
        let mut state = self.lock();
        let fresh = state.entries.get(hash).map(|e| self.is_fresh(e, now_ms))?;
        if !fresh {
            state.entries.remove(hash);
            return None;
        }
        state.entries.get(hash).map(|e| e.session.clone())
    }

    pub fn generation(&self) -> CacheGeneration {
        CacheGeneration(self.lock().generation)
    }

    /// Caches `session` unless an invalidation happened since `seen` was taken. Returns
    /// whether it was stored.
    pub(super) fn insert(
        &self,
        hash: TokenHash,
        session: CachedSession,
        seen: CacheGeneration,
        now_ms: i64,
    ) -> bool {
        let mut state = self.lock();
        if state.generation != seen.0 {
            return false;
        }
        if state.entries.len() >= self.capacity && !state.entries.contains_key(&hash) {
            self.make_room(&mut state, now_ms);
        }
        let entry = Entry {
            session,
            cached_at_ms: now_ms,
        };
        state.entries.insert(hash, entry);
        true
    }

    /// Records a use written to the database (sliding idle expiry).
    pub(super) fn mark_used(&self, hash: &TokenHash, last_used_at_ms: i64) {
        if let Some(entry) = self.lock().entries.get_mut(hash) {
            entry.session.last_used_at_ms = last_used_at_ms;
        }
    }

    /// Drops an entry whose session expired. Unlike the `invalidate_*` methods it leaves
    /// concurrent fills alone: a re-filled expired session is rejected by its check anyway.
    pub(super) fn remove_expired(&self, hash: &TokenHash) {
        self.lock().entries.remove(hash);
    }

    pub fn invalidate_token(&self, hash: &TokenHash) {
        self.invalidate(|state| {
            state.entries.remove(hash);
        });
    }

    pub fn invalidate_session(&self, session_id: i64) {
        self.invalidate(|state| {
            state
                .entries
                .retain(|_, e| e.session.principal.session_id != session_id);
        });
    }

    pub fn invalidate_user(&self, user_id: i64) {
        self.invalidate(|state| {
            state
                .entries
                .retain(|_, e| e.session.principal.user_id != user_id);
        });
    }

    pub fn flush(&self) {
        self.invalidate(|state| state.entries.clear());
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn invalidate(&self, change: impl FnOnce(&mut State)) {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1);
        change(&mut state);
    }

    fn make_room(&self, state: &mut State, now_ms: i64) {
        state.entries.retain(|_, e| self.is_fresh(e, now_ms));
        if state.entries.len() >= self.capacity {
            let victim = state.entries.keys().next().cloned();
            if let Some(victim) = victim {
                state.entries.remove(&victim);
            }
        }
    }

    fn is_fresh(&self, entry: &Entry, now_ms: i64) -> bool {
        now_ms.saturating_sub(entry.cached_at_ms) < self.ttl_ms
    }

    /// A panic while holding the lock cannot leave the map half-updated (every critical
    /// section is a single map operation), so poisoning is ignored.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use deskhaven_db::entity::user::Role;

    use super::super::SessionKind;
    use super::*;

    const TTL_MS: i64 = 60_000;

    fn hash(n: u8) -> TokenHash {
        TokenHash::of_presented(&char::from(b'A' + n).to_string().repeat(43)).unwrap()
    }

    fn session(session_id: i64, user_id: i64) -> CachedSession {
        CachedSession {
            principal: SessionPrincipal {
                session_id,
                user_id,
                role: Role::User,
                kind: SessionKind::Console,
                device: None,
            },
            expires_at_ms: 1_000_000,
            last_used_at_ms: 0,
        }
    }

    fn cache(capacity: usize) -> SessionCache {
        SessionCache::new(Duration::from_millis(60_000), capacity)
    }

    #[test]
    fn entries_serve_until_the_ttl_then_disappear() {
        let cache = cache(8);
        let g = cache.generation();
        assert!(cache.insert(hash(0), session(1, 1), g, 0));

        assert_eq!(cache.get(&hash(0), TTL_MS - 1), Some(session(1, 1)));
        assert_eq!(cache.get(&hash(0), TTL_MS), None);
        assert!(cache.is_empty());
    }

    #[test]
    fn invalidation_by_token_session_user_and_flush() {
        let cache = cache(8);
        let g = cache.generation();
        cache.insert(hash(0), session(1, 10), g, 0);
        cache.insert(hash(1), session(2, 10), g, 0);
        cache.insert(hash(2), session(3, 20), g, 0);
        cache.insert(hash(3), session(4, 30), g, 0);

        cache.invalidate_token(&hash(0));
        assert_eq!(cache.get(&hash(0), 1), None);
        cache.invalidate_session(4);
        assert_eq!(cache.get(&hash(3), 1), None);
        cache.invalidate_user(10);
        assert_eq!(cache.get(&hash(1), 1), None);
        assert!(cache.get(&hash(2), 1).is_some());
        cache.flush();
        assert!(cache.is_empty());
    }

    #[test]
    fn a_fill_that_raced_an_invalidation_is_dropped() {
        let cache = cache(8);
        let seen = cache.generation();

        cache.invalidate_user(99);
        let stored = cache.insert(hash(0), session(1, 1), seen, 0);

        assert!(!stored);
        assert_eq!(cache.get(&hash(0), 1), None);
        assert!(cache.insert(hash(0), session(1, 1), cache.generation(), 0));
    }

    #[test]
    fn capacity_is_a_hard_bound_and_stale_entries_go_first() {
        let cache = cache(2);
        cache.insert(hash(0), session(1, 1), cache.generation(), 0);
        cache.insert(hash(1), session(2, 1), cache.generation(), TTL_MS / 2);

        // hash(0) is stale at this time: it is the one purged.
        cache.insert(hash(2), session(3, 1), cache.generation(), TTL_MS + 1);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&hash(1), TTL_MS + 1).is_some());

        // All fresh: one arbitrary entry makes room.
        cache.insert(hash(3), session(4, 1), cache.generation(), TTL_MS + 2);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&hash(3), TTL_MS + 2).is_some());
    }

    #[test]
    fn mark_used_slides_the_idle_window() {
        let cache = cache(8);
        cache.insert(hash(0), session(1, 1), cache.generation(), 0);

        cache.mark_used(&hash(0), 500);

        assert_eq!(cache.get(&hash(0), 1).unwrap().last_used_at_ms, 500);
    }

    #[test]
    fn check_enforces_absolute_and_idle_expiry() {
        let s = CachedSession {
            expires_at_ms: 10_000,
            last_used_at_ms: 1_000,
            ..session(1, 1)
        };

        assert!(s.check(5_000, 5_000).is_ok());
        assert!(matches!(s.check(6_000, 5_000), Err(SessionError::Expired)));
        assert!(matches!(
            s.check(10_000, 100_000),
            Err(SessionError::Expired)
        ));
    }
}
