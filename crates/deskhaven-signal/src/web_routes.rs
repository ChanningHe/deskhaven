//! Routes of in-process web punches (deskhaven code, not vendored; s4): who may answer a punch
//! made by [`SignalHandle::web_punch`](crate::SignalHandle::web_punch), and the trickle ICE of
//! its WebRTC session in both directions. Independent implementation; wire behavior and limits
//! follow rustdesk-server PR #734 (39b1b3f).
//!
//! Only `web_punch` opens routes, keyed by the controller address its PunchHole carries (a
//! claimed address, `claimed_addr.rs`, held for the route's life). A wire client can only use a
//! route by answering as its device: from the device's registered IP, with its id (answers) or
//! the offer's session key (candidates).

use hbb_common::{
    rendezvous_proto::{IceCandidate, RendezvousMessage},
    serde_json,
    tokio::{sync::mpsc, time::Instant},
    try_into_v4,
};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

/// Largest WebRTC offer / answer endpoint passed on.
pub(crate) const MAX_SDP_LEN: usize = 4096;
/// Largest ICE candidate passed on.
pub(crate) const MAX_CANDIDATE_LEN: usize = 2048;
/// Largest session key (the offer's DTLS fingerprint).
const MAX_SESSION_KEY_LEN: usize = 256;
/// Candidates a route passes on per direction.
pub(crate) const MAX_ICE_PER_DIRECTION: u32 = 128;
/// IceCandidate frames one stream connection may send.
const MAX_ICE_FRAMES: u32 = 256;
/// IceCandidate frames without a route one stream connection may send.
const MAX_ICE_MISSES: u32 = 16;
/// How long a web punch route lives, from the punch.
pub const WEB_ROUTE_TTL: Duration = Duration::from_secs(60);
/// Routes alive at once, in total and per browser IP.
const MAX_ROUTES: usize = 10_000;
const MAX_ROUTES_PER_IP: usize = 64;

/// One web punch: its device, and its WebRTC session if an offer went out.
pub(crate) struct PunchRoute {
    pub(crate) peer_id: String,
    /// The device's registered IP at the punch: answers and candidates must come from it.
    pub(crate) peer_ip: IpAddr,
    /// The offer's DTLS fingerprint (`a=fingerprint` value), `None` without an offer.
    pub(crate) session_key: Option<String>,
    pub(crate) to_peer: u32,
    pub(crate) to_ctl: u32,
    pub(crate) expires: Instant,
    /// The device's RelayResponse went to the controller: it takes only one.
    pub(crate) answered: bool,
    /// The punch's in-process controller: the device's answer and candidates go here, never
    /// through `tcp_punch`, whose keys wire connections also use.
    pub(crate) to_ctl_tx: mpsc::Sender<RendezvousMessage>,
}

impl PunchRoute {
    fn alive(&self, now: Instant) -> bool {
        now < self.expires
    }

    fn webrtc(&self, session_key: &str, now: Instant) -> bool {
        self.alive(now) && self.session_key.as_deref() == Some(session_key)
    }
}

/// What a RelayResponse for a controller address may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// No web punch route: a wire controller's answer.
    Unrouted,
    /// From the route's device: passed on.
    Accept,
    /// For a route, but not from its device: dropped.
    Refuse,
}

/// What happened to a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ice {
    Forwarded,
    /// The route's budget for that direction is used up: dropped quietly.
    OverBudget,
    /// Malformed, no live route, another session or another host.
    NoMatch,
}

/// Why [`WebRoute::send_candidate`](crate::WebRoute::send_candidate) refused a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CandidateError {
    /// The route has no WebRTC session, expired, or the server stopped.
    #[error("the route is closed")]
    Closed,
    /// Empty or longer than 2048 bytes.
    #[error("invalid ICE candidate")]
    Invalid,
    /// The route already passed 128 candidates to the device.
    #[error("too many ICE candidates for this route")]
    OverBudget,
}

/// The live web punch routes of one server. The lock is never held across an await.
#[derive(Clone, Default)]
pub(crate) struct WebRoutes(Arc<Mutex<HashMap<SocketAddr, PunchRoute>>>);

impl WebRoutes {
    fn lock(&self) -> MutexGuard<'_, HashMap<SocketAddr, PunchRoute>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds the route of controller address `ctl`; `false` when the table or `ctl`'s IP is at
    /// its cap, or `ctl` has a live route (cannot happen while claims are unique).
    pub(crate) fn open(&self, ctl: SocketAddr, route: PunchRoute, now: Instant) -> bool {
        let ctl = try_into_v4(ctl);
        let mut map = self.lock();
        map.retain(|_, r| r.alive(now));
        let same_ip = map.keys().filter(|k| k.ip() == ctl.ip()).count();
        if map.len() >= MAX_ROUTES || same_ip >= MAX_ROUTES_PER_IP || map.contains_key(&ctl) {
            return false;
        }
        map.insert(ctl, route);
        true
    }

    pub(crate) fn remove(&self, ctl: SocketAddr) {
        self.lock().remove(&try_into_v4(ctl));
    }

    /// Drops expired routes, which closes their controllers' candidate channels.
    pub(crate) fn sweep(&self, now: Instant) {
        self.lock().retain(|_, r| r.alive(now));
    }

    /// Whether a RelayResponse naming `id`, sent from `from`, may reach controller `ctl`.
    pub(crate) fn reply(&self, ctl: SocketAddr, id: &str, from: IpAddr, now: Instant) -> Reply {
        match self.lock().get(&try_into_v4(ctl)) {
            None => Reply::Unrouted,
            Some(r) if r.alive(now) && r.peer_id == id && same_ip(r.peer_ip, from) => Reply::Accept,
            Some(_) => Reply::Refuse,
        }
    }

    /// Passes the device's RelayResponse (`reply` accepted it) to controller `ctl`, once.
    pub(crate) fn answer(&self, ctl: SocketAddr, msg: RendezvousMessage) -> bool {
        let mut map = self.lock();
        let Some(r) = map.get_mut(&try_into_v4(ctl)).filter(|r| !r.answered) else {
            return false;
        };
        r.answered = true;
        r.to_ctl_tx.try_send(msg).is_ok()
    }

    /// Passes a candidate the device sent from `from` (`trusted`: on a KeyExchange-secured TCP
    /// connection) to its route's controller.
    pub(crate) fn to_controller(
        &self,
        ice: IceCandidate,
        from: IpAddr,
        trusted: bool,
        now: Instant,
    ) -> Ice {
        if !trusted || !valid_candidate(&ice.candidate) || !valid_key(&ice.session_key) {
            return Ice::NoMatch;
        }
        let Some(ctl) = crate::addr_mangle::decode(&ice.socket_addr) else {
            return Ice::NoMatch;
        };
        let mut map = self.lock();
        let Some(r) = map.get_mut(&try_into_v4(ctl)) else {
            return Ice::NoMatch;
        };
        if !r.webrtc(&ice.session_key, now) || !same_ip(r.peer_ip, from) {
            return Ice::NoMatch;
        }
        if r.to_ctl >= MAX_ICE_PER_DIRECTION {
            return Ice::OverBudget;
        }
        r.to_ctl += 1;
        // The channel holds the whole budget; it only refuses once its WebRoute is gone.
        let _ = r.to_ctl_tx.try_send(candidate_msg(String::new(), ice));
        Ice::Forwarded
    }

    /// The device id and the message carrying `candidate` from controller `ctl` to it.
    pub(crate) fn to_peer(
        &self,
        ctl: SocketAddr,
        candidate: String,
        now: Instant,
    ) -> Result<(String, RendezvousMessage), CandidateError> {
        if !valid_candidate(&candidate) {
            return Err(CandidateError::Invalid);
        }
        let mut map = self.lock();
        let r = map
            .get_mut(&try_into_v4(ctl))
            .filter(|r| r.alive(now))
            .ok_or(CandidateError::Closed)?;
        let session_key = r.session_key.clone().ok_or(CandidateError::Closed)?;
        if r.to_peer >= MAX_ICE_PER_DIRECTION {
            return Err(CandidateError::OverBudget);
        }
        r.to_peer += 1;
        let ice = IceCandidate {
            session_key,
            candidate,
            ..Default::default()
        };
        Ok((r.peer_id.clone(), candidate_msg(r.peer_id.clone(), ice)))
    }
}

fn candidate_msg(id: String, ice: IceCandidate) -> RendezvousMessage {
    let mut msg = RendezvousMessage::new();
    msg.set_ice_candidate(IceCandidate {
        id,
        session_key: ice.session_key,
        candidate: ice.candidate,
        ..Default::default()
    });
    msg
}

fn valid_candidate(candidate: &str) -> bool {
    !candidate.is_empty() && candidate.len() <= MAX_CANDIDATE_LEN
}

fn valid_key(session_key: &str) -> bool {
    !session_key.is_empty() && session_key.len() <= MAX_SESSION_KEY_LEN
}

fn same_ip(a: IpAddr, b: IpAddr) -> bool {
    a.to_canonical() == b.to_canonical()
}

/// IceCandidate frames seen on one stream connection.
#[derive(Default)]
pub(crate) struct IceCount {
    frames: u32,
    misses: u32,
}

impl IceCount {
    /// Handles a candidate received on the connection; `false` closes the connection (more
    /// than 256 candidate frames, or more than 16 that matched no route).
    pub(crate) fn on_candidate(
        &mut self,
        routes: &WebRoutes,
        ice: IceCandidate,
        from: IpAddr,
        trusted: bool,
    ) -> bool {
        self.frames += 1;
        if self.frames > MAX_ICE_FRAMES {
            return false;
        }
        match routes.to_controller(ice, from, trusted, Instant::now()) {
            Ice::Forwarded | Ice::OverBudget => true,
            Ice::NoMatch => {
                self.misses += 1;
                self.misses <= MAX_ICE_MISSES
            }
        }
    }
}

/// The session key of a WebRTC offer endpoint (`webrtc://` + base64 of the JSON session
/// description): its SDP's session-level `a=fingerprint` value, else the first one of an
/// `application` media section, as the device derives it (hbb_common webrtc.rs
/// `get_key_for_sdp`). `None` for anything else, or beyond the size limits.
pub(crate) fn offer_session_key(offer: &str) -> Option<String> {
    if offer.len() > MAX_SDP_LEN {
        return None;
    }
    let json = base64::decode(offer.strip_prefix("webrtc://")?).ok()?;
    let desc: serde_json::Value = serde_json::from_slice(&json).ok()?;
    if desc.get("type")?.as_str()? != "offer" {
        return None;
    }
    let key = sdp_fingerprint(desc.get("sdp")?.as_str()?)?;
    valid_key(key).then(|| key.to_owned())
}

fn sdp_fingerprint(sdp: &str) -> Option<&str> {
    let mut application = None;
    let mut found = None;
    for line in sdp.lines() {
        if let Some(media) = line.strip_prefix("m=") {
            application = Some(media.split(' ').next() == Some("application"));
        } else if let Some(fp) = line.strip_prefix("a=fingerprint:") {
            match application {
                None => return Some(fp),
                Some(true) if found.is_none() => found = Some(fp),
                Some(_) => {}
            }
        }
    }
    found
}

#[cfg(test)]
#[path = "web_routes/tests.rs"]
mod tests;
