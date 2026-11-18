// Modified by deskhaven; see crates/deskhaven-signal/CHANGES.md (AGPL-3.0 §5a).
use crate::common::*;
use crate::ip_blocker::IpBlocker;
use crate::log_limit::Redacted;
use crate::persist::{Persist, Refused, SaveState, WRITE_QUEUE};
use crate::store::{PeerStore, StoreError};
use crate::tasks::Tasks;
use hbb_common::{
    bytes::Bytes,
    log,
    regex::Regex,
    tokio::{
        sync::{Mutex, RwLock},
        time::Duration,
    },
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, LazyLock},
    time::Instant,
};

// deskhaven (M4-05): the map behind IP_BLOCKER is `ip_blocker::IpBlocker` (sources keyed by
// IPv4 address / IPv6 /64, id hashes, eviction instead of a lockout when full).
#[allow(dead_code)] // only used by the unused USER_STATUS static (upstream)
type UserStatusMap = HashMap<Vec<u8>, Arc<(Option<Vec<u8>>, bool)>>;
lazy_static::lazy_static! {
    pub(crate) static ref IP_BLOCKER: Mutex<IpBlocker> = Default::default();
    pub(crate) static ref USER_STATUS: RwLock<UserStatusMap> = Default::default();
}
// deskhaven: IP_CHANGES, IpChangesMap, IP_CHANGE_DUR and IP_CHANGE_DUR_X2 removed: the map was
// only read by the removed console command `ic`, so it was write-only state.
pub const DAY_SECONDS: u64 = 3600 * 24;
pub const IP_BLOCK_DUR: u64 = 60;
// Same bound as the Pro server: a longer id is not a normal RustDesk id or hostname,
// and every peer entry and punch-request record keeps its own copy.
pub const MAX_PEER_ID_LEN: usize = 253;

/// The client's custom id rule: `hbb_common::is_valid_custom_id` (hbb_common fae47e0 lib.rs),
/// the check behind the client's "Change ID" and `--set-id`. That function builds the regex
/// on every call (milliseconds, too slow for unauthenticated packets), so it is compiled once
/// here from the same pattern with the same `regex` (hbb_common's re-export); a test checks
/// that both agree.
const CLIENT_CUSTOM_ID: &str = r"^[a-zA-Z][\w-]{5,15}$";

/// `None` only if the pattern did not compile, which the tests rule out.
static CLIENT_CUSTOM_ID_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(CLIENT_CUSTOM_ID).ok());

/// deskhaven: whether `id` may be registered or stored: 1..=MAX_PEER_ID_LEN bytes that are
/// either
/// - ASCII letters, digits, `-` and `_` (RustDesk's numeric ids, ASCII custom ids, legacy ids),
///   or
/// - a custom id the client itself accepts ([`CLIENT_CUSTOM_ID`], Unicode `\w`), e.g.
///   `aé12345` or `a张三1234`, minus the Join_Control characters ZWNJ / ZWJ, which `\w`
///   includes but which are invisible (an id that looks like another).
///
/// Refusing an id the client accepts is not harmless: RegisterPk answers UUID_MISMATCH and the
/// client silently replaces its id with a random one. Everything else stays out: NUL (refused
/// by PostgreSQL text, so the write would be retried forever), control characters and line
/// breaks (forged log lines), whitespace, bidi and other format characters (Cf, not `\w`).
///
/// The other id rules are narrower, not identical: the client API's sign-in
/// (deskhaven-client-api login/request.rs `is_rustdesk_id`) takes the ASCII set only (up to
/// 128 bytes), so a device with a non-ASCII custom id registers but cannot sign in; the
/// console's deploy (deskhaven-core devices/service/deploy.rs `is_deployable_id`) takes the
/// ASCII set within RegisterPk's 6..=MAX_PEER_ID_LEN bytes.
pub(crate) fn is_peer_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_PEER_ID_LEN && (is_ascii_id(id) || is_client_custom_id(id))
}

fn is_ascii_id(id: &str) -> bool {
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_client_custom_id(id: &str) -> bool {
    !id.contains(is_join_control)
        && CLIENT_CUSTOM_ID_RE
            .as_ref()
            .is_some_and(|re| re.is_match(id))
}

/// Unicode Join_Control: ZERO WIDTH NON-JOINER and ZERO WIDTH JOINER.
fn is_join_control(c: char) -> bool {
    matches!(c, '\u{200c}' | '\u{200d}')
}

// deskhaven: no serde any more; the info JSON column of upstream's `peer` table is gone
// (the store keeps the IP itself).
#[derive(Debug, Default, Clone)]
pub(crate) struct PeerInfo {
    pub(crate) ip: String,
}

pub(crate) struct Peer {
    pub(crate) socket_addr: SocketAddr,
    pub(crate) last_reg_time: Instant,
    // deskhaven: `guid` removed: it only told insert from update, which `upsert_pk` decides.
    pub(crate) uuid: Bytes,
    pub(crate) pk: Bytes,
    // pub(crate) user: Option<Vec<u8>>,
    pub(crate) info: PeerInfo,
    // pub(crate) disabled: bool,
    pub(crate) reg_pk: (u32, Instant), // how often register_pk
    // deskhaven: write-behind bookkeeping (persist.rs).
    pub(crate) save: SaveState,
}

impl Default for Peer {
    fn default() -> Self {
        Self {
            socket_addr: "0.0.0.0:0".parse().unwrap(),
            last_reg_time: get_expired_time(),
            uuid: Bytes::new(),
            pk: Bytes::new(),
            info: Default::default(),
            // user: None,
            // disabled: false,
            reg_pk: (0, get_expired_time()),
            save: SaveState::default(),
        }
    }
}

pub(crate) type LockPeer = Arc<RwLock<Peer>>;
pub(crate) type PeerCache = Arc<RwLock<HashMap<String, LockPeer>>>;

// deskhaven: memory is authoritative while serving (upstream: a cache in front of SQLite).
// It is filled from the injected `PeerStore` before the listeners start (`preload`), so
// lookups never touch the store; accepted registrations are written behind (persist.rs).
#[derive(Clone)]
pub(crate) struct PeerMap {
    map: PeerCache,
    persist: Persist,
}

impl PeerMap {
    // deskhaven: upstream resolved DB_URL and opened SQLite here.
    pub(crate) fn new(store: Arc<dyn PeerStore>, tasks: &Tasks) -> Self {
        Self::with_write_queue(store, tasks, WRITE_QUEUE)
    }

    // deskhaven: `cap` bounds the write queue for unbound ids (persist.rs).
    pub(crate) fn with_write_queue(store: Arc<dyn PeerStore>, tasks: &Tasks, cap: usize) -> Self {
        let map = PeerCache::default();
        let persist = Persist::start(store, map.clone(), tasks, cap);
        Self { map, persist }
    }

    // deskhaven: loads every stored registration (startup, before serving).
    pub(crate) async fn preload(&self, limit: Duration) -> Result<usize, StoreError> {
        self.persist.preload(limit).await
    }

    // deskhaven: the store write is queued (coalesced per id), not awaited. Fails, before
    // touching the peer, only for an id that is not bound yet when the queue is full (or one
    // that is not a peer id, which the caller already refused); the caller then refuses the
    // registration. Upstream awaited the database and ignored the result.
    // deskhaven (M1-07 review, H1): a refused id also loses the default entry `get_or` made
    // for it, so no peer without a key stays behind (it answered punch holes as OFFLINE and
    // had its empty pk signed). Removed under the peer's lock (lock order peer -> map; nothing
    // waits for a peer while holding the map), and only if the entry is still this peer.
    #[inline]
    pub(crate) async fn update_pk(
        &mut self,
        id: String,
        peer: LockPeer,
        addr: SocketAddr,
        uuid: Bytes,
        pk: Bytes,
        ip: String,
    ) -> Result<(), Refused> {
        // deskhaven (M2-12 review): the uuid is never logged (proof of identity for the
        // device endpoints), only its length.
        log::info!(
            "update_pk {:?} {:?} uuid {} {:?}",
            id,
            addr,
            Redacted(&uuid),
            pk
        );
        let mut w = peer.write().await;
        // Queued under the peer's lock: the writer reads the peer after this change.
        if let Err(full) = self.persist.enqueue(&id, w.uuid.is_empty()) {
            let mut map = self.map.write().await;
            if w.uuid.is_empty() && map.get(&id).is_some_and(|p| Arc::ptr_eq(p, &peer)) {
                map.remove(&id);
            }
            return Err(full);
        }
        w.socket_addr = addr;
        w.uuid = uuid;
        w.pk = pk;
        w.last_reg_time = Instant::now();
        w.info.ip = ip;
        w.save.accepted();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn persist(&self) -> &Persist {
        &self.persist
    }

    // deskhaven: memory only; after the preload a miss means the id is not registered.
    #[inline]
    pub(crate) async fn get(&self, id: &str) -> Option<LockPeer> {
        self.get_in_memory(id).await
    }

    #[inline]
    pub(crate) async fn get_or(&self, id: &str) -> LockPeer {
        if let Some(p) = self.get(id).await {
            return p;
        }
        let mut w = self.map.write().await;
        if let Some(p) = w.get(id) {
            return p.clone();
        }
        let tmp = LockPeer::default();
        w.insert(id.to_owned(), tmp.clone());
        tmp
    }

    #[inline]
    pub(crate) async fn get_in_memory(&self, id: &str) -> Option<LockPeer> {
        self.map.read().await.get(id).cloned()
    }

    #[inline]
    #[allow(dead_code)] // unused upstream
    pub(crate) async fn is_in_memory(&self, id: &str) -> bool {
        self.map.read().await.contains_key(id)
    }

    // deskhaven: invalidation hook for out-of-band registry changes: `SignalHandle::reload_peer`
    // (M2-12, device deletion); the writer also calls `Persist::reload` on UuidMismatch.
    pub(crate) async fn reload(&self, id: &str) -> Result<(), StoreError> {
        self.persist.reload(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // deskhaven: the ASCII id set up to MAX_PEER_ID_LEN, plus every custom id the client
    // accepts (Unicode `\w`) except those with Join_Control characters.
    #[test]
    fn peer_ids_are_ascii_ids_or_client_custom_ids_within_the_length_cap() {
        let at_cap = "1".repeat(MAX_PEER_ID_LEN);
        let over_cap = "1".repeat(MAX_PEER_ID_LEN + 1);
        let valid = [
            "123456789",
            "abc_def-12",
            "Custom-id_123456",
            "a",
            at_cap.as_str(),
            "a\u{e9}12345",
            "a\u{5f20}\u{4e09}1234",
            "a\u{203f}bcdef",
            "abc\u{e9}def",
        ];
        let invalid = [
            "",
            "10\0aaa",
            "a\0bcdef",
            "abc\ndef",
            "abc\rdef",
            "abc\u{200b}def",
            "abc\u{200c}def",
            "abc\u{200d}def",
            "abc\u{202e}def",
            "abc\u{2066}def",
            "abc\u{feff}def",
            "abc def",
            "abc\u{a0}def",
            "abc\u{3000}def",
            "abc.def",
            "abc@def",
            over_cap.as_str(),
            // Outside the client's custom id rule: first character not an ASCII letter, or
            // fewer than 6 / more than 16 characters.
            "\u{e9}bcdef",
            "\u{ff21}bcdef",
            "1\u{e9}cdef",
            "a\u{e9}cd",
            "a\u{e9}cdefghijklmnopq",
        ];

        for id in valid {
            assert!(is_peer_id(id), "rejected {id:?}");
        }
        for id in invalid {
            assert!(!is_peer_id(id), "accepted {id:?}");
        }
    }

    // deskhaven: the compiled custom id rule is the client's own check (hbb_common's
    // `is_valid_custom_id`), and the non-ASCII arm of `is_peer_id` is that rule minus
    // Join_Control.
    #[test]
    fn non_ascii_peer_ids_follow_the_client_custom_id_rule() {
        let ids = [
            "abcdef",
            "abcdefghijklmnop",
            "abcdefghijklmnopq",
            "abcde",
            "1abcdef",
            "abc_de-f",
            "a\u{e9}12345",
            "a\u{5f20}\u{4e09}1234",
            "a\u{203f}bcdef",
            "a\u{0301}bcdef",
            "a\u{0663}bcdef",
            "\u{ff21}bcdef",
            "a\u{e9}cd",
            "a\u{e9}cdefghijklmnopq",
            "abc\u{200b}def",
            "abc\u{202e}def",
            "abc\u{feff}def",
            "abc def",
            "abc\0def",
            "abc\ndef",
            "abc.def",
        ];

        assert!(CLIENT_CUSTOM_ID_RE.is_some());
        for id in ids {
            let client = hbb_common::is_valid_custom_id(id);
            assert_eq!(is_client_custom_id(id), client, "{id:?}");
            if !id.is_ascii() {
                assert_eq!(is_peer_id(id), client, "{id:?}");
            }
        }
        for id in ["abc\u{200c}def", "abc\u{200d}def"] {
            assert!(hbb_common::is_valid_custom_id(id), "{id:?}");
            assert!(!is_peer_id(id), "{id:?}");
        }
    }
}
