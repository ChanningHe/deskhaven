//! Web punch routes. A wire client can only use a route by answering as its device: from the
//! device's registered IP, with its id (answers) or the offer's session key (candidates).

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

pub(crate) const MAX_SDP_LEN: usize = 4096;
pub(crate) const MAX_CANDIDATE_LEN: usize = 2048;
/// Largest session key (the offer's DTLS fingerprint).
const MAX_SESSION_KEY_LEN: usize = 256;
pub(crate) const MAX_ICE_PER_DIRECTION: u32 = 128;
const MAX_ICE_FRAMES: u32 = 256;
const MAX_ICE_MISSES: u32 = 16;
pub const WEB_ROUTE_TTL: Duration = Duration::from_secs(60);
/// Routes alive at once, in total and per browser IP.
const MAX_ROUTES: usize = 10_000;
const MAX_ROUTES_PER_IP: usize = 64;

pub(crate) struct PunchRoute {
    pub(crate) peer_id: String,
    /// The device's registered IP at the punch: answers and candidates must come from it.
    pub(crate) peer_ip: IpAddr,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    Unrouted,
    Accept,
    Refuse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ice {
    Forwarded,
    OverBudget,
    NoMatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CandidateError {
    #[error("the route is closed")]
    Closed,
    #[error("invalid ICE candidate")]
    Invalid,
    #[error("too many ICE candidates for this route")]
    OverBudget,
}

/// The lock is never held across an await.
#[derive(Clone, Default)]
pub(crate) struct WebRoutes(Arc<Mutex<HashMap<SocketAddr, PunchRoute>>>);

impl WebRoutes {
    fn lock(&self) -> MutexGuard<'_, HashMap<SocketAddr, PunchRoute>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

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

    pub(crate) fn sweep(&self, now: Instant) {
        self.lock().retain(|_, r| r.alive(now));
    }

    pub(crate) fn reply(&self, ctl: SocketAddr, id: &str, from: IpAddr, now: Instant) -> Reply {
        match self.lock().get(&try_into_v4(ctl)) {
            None => Reply::Unrouted,
            Some(r) if r.alive(now) && r.peer_id == id && same_ip(r.peer_ip, from) => Reply::Accept,
            Some(_) => Reply::Refuse,
        }
    }

    pub(crate) fn answer(&self, ctl: SocketAddr, msg: RendezvousMessage) -> bool {
        let mut map = self.lock();
        let Some(r) = map.get_mut(&try_into_v4(ctl)).filter(|r| !r.answered) else {
            return false;
        };
        r.answered = true;
        r.to_ctl_tx.try_send(msg).is_ok()
    }

    /// `trusted`: the candidate came on a KeyExchange-secured TCP connection.
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

#[derive(Default)]
pub(crate) struct IceCount {
    frames: u32,
    misses: u32,
}

impl IceCount {
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

/// The SDP's session-level `a=fingerprint`, else the first one of an `application` media
/// section, as the device derives it.
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
