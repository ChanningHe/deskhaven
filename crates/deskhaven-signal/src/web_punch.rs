//! In-process web punch (deskhaven code, not vendored; s4): the server asks a device for a
//! relay session, and with a browser offer for a WebRTC answer, on behalf of a caller in this
//! process (the console's web client); wire clients cannot start one. Independent
//! implementation; wire behavior and limits follow rustdesk-server PR #734 (39b1b3f).
//!
//! One PunchHole gets both transports: it forces relay (`nat_type` SYMMETRIC) to the
//! deployment's first relay and carries the offer, and the device answers with a RelayResponse
//! holding its relay uuid and, if it made one, the WebRTC answer. Answers and candidates are
//! checked against the punch's route (`web_routes.rs`).

use crate::claimed_addr::ClaimedAddr;
use crate::rendezvous_server::{Data, REG_TIMEOUT, RendezvousServer, Sender};
use crate::web_routes::{CandidateError, PunchRoute, WebRoutes, offer_session_key};
use hbb_common::{
    AddrMangle,
    rendezvous_proto::{NatType, PunchHole, RelayResponse, RendezvousMessage, rendezvous_message},
    tokio::{
        self,
        sync::mpsc,
        time::{Instant, timeout_at},
    },
    try_into_v4,
};
use std::{
    collections::VecDeque,
    net::{IpAddr, SocketAddr},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

/// How long [`SignalHandle::web_punch`](crate::SignalHandle::web_punch) waits for the device.
pub const WEB_PUNCH_TIMEOUT: Duration = Duration::from_secs(15);
/// The route's channel to its controller: one RelayResponse and the device's candidate budget.
const LOCAL_SINK_LEN: usize = crate::web_routes::MAX_ICE_PER_DIRECTION as usize + 1;
/// An ed25519 public key, what a registered device must hold to be vouched for.
const DEVICE_PK_LEN: usize = 32;

/// A punch on behalf of a browser.
#[derive(Clone, Debug)]
pub struct WebPunch {
    /// The device id.
    pub target: String,
    /// The browser's WebRTC offer endpoint (`webrtc://` + base64 of the JSON session
    /// description, at most 4096 bytes); `None` for relay only. Not sent while
    /// `always_use_relay` is on.
    pub offer: Option<String>,
    /// The browser's IP as the HTTP stack resolved it (trusted proxies applied). The device
    /// sees it, with a port unique to this punch, as the controller's address.
    pub client: IpAddr,
}

/// Why a web punch failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WebPunchError {
    /// No device with this id is registered.
    #[error("device not found")]
    NotFound,
    /// The device has not been seen for 30 s.
    #[error("device is offline")]
    Offline,
    /// The device sent no usable answer within [`WEB_PUNCH_TIMEOUT`].
    #[error("the device did not answer")]
    NoAnswer,
    /// Too many web punches in flight (in total or from this browser IP).
    #[error("too many web punches in flight")]
    Busy,
    /// The signal server is not running.
    #[error("signal server is not running")]
    Stopped,
    /// The device relays through its own relay server (its local `relay-server` option),
    /// which the browser cannot reach through this deployment.
    #[error("the device uses its own relay server {relay_server:?}")]
    ForeignRelay { relay_server: String },
    /// No relay server is configured (or none resolved).
    #[error("no relay server is configured")]
    NoRelay,
    /// The offer is not a WebRTC offer endpoint within the size limit.
    #[error("invalid WebRTC offer")]
    InvalidOffer,
}

/// An answered web punch. Its route lives until [`WEB_ROUTE_TTL`](crate::WEB_ROUTE_TTL) passes
/// or both candidate halves ([`WebRoute::split`]) are dropped; from then on the device's
/// candidates are dropped. A route without a WebRTC answer is revoked at once.
pub struct WebRoute {
    /// The relay session's uuid (for `RequestRelay` to the relay server).
    pub uuid: String,
    /// The relay the device connects to: the deployment's first relay server.
    pub relay_server: String,
    /// `RelayResponse.pk` as the server sent it: `IdPk{id, pk}` signed with the server key,
    /// for the browser to verify before trusting the device key.
    pub signed_pk: Vec<u8>,
    /// The device's WebRTC answer endpoint, if an offer went out and it answered one.
    pub answer: Option<String>,
    sender: CandidateSender,
    receiver: CandidateReceiver,
}

/// The browser's half of a route's trickle ICE; clones share the route.
#[derive(Clone)]
pub struct CandidateSender(Option<SendHalf>);

#[derive(Clone)]
struct SendHalf {
    link: Arc<RouteLink>,
    server: Sender,
}

/// The device's half of a route's trickle ICE.
pub struct CandidateReceiver(Option<RecvHalf>);

struct RecvHalf {
    _link: Arc<RouteLink>,
    rx: mpsc::Receiver<RendezvousMessage>,
    /// Candidates that arrived before the RelayResponse.
    pending: VecDeque<String>,
    expires: Instant,
    server: Sender,
}

/// Holds a route open; removing it on drop closes its candidate channel. The claim outlives the
/// route (fields drop after `drop`), so its address never names another route meanwhile.
struct RouteLink {
    routes: WebRoutes,
    ctl: SocketAddr,
    _claim: ClaimedAddr,
}

impl Drop for RouteLink {
    fn drop(&mut self) {
        self.routes.remove(self.ctl);
    }
}

impl std::fmt::Debug for WebRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebRoute")
            .field("uuid", &self.uuid)
            .field("relay_server", &self.relay_server)
            .field("answer", &self.answer.is_some())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for CandidateSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CandidateSender").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for CandidateReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CandidateReceiver").finish_non_exhaustive()
    }
}

impl WebRoute {
    /// See [`CandidateSender::send`].
    pub fn send_candidate(&self, candidate: String) -> Result<(), CandidateError> {
        self.sender.send(candidate)
    }

    /// See [`CandidateReceiver::next`].
    pub async fn next_candidate(&mut self) -> Option<String> {
        self.receiver.next().await
    }

    /// The two halves of the trickle ICE, so that waiting for the device's next candidate
    /// never holds up sending the browser's.
    pub fn split(self) -> (CandidateSender, CandidateReceiver) {
        (self.sender, self.receiver)
    }
}

impl CandidateSender {
    /// Sends one of the browser's ICE candidates to the device (through its registration
    /// channel, UDP or TCP). At most 128 per route, each at most 2048 bytes.
    pub fn send(&self, candidate: String) -> Result<(), CandidateError> {
        let half = self.0.as_ref().ok_or(CandidateError::Closed)?;
        let (id, msg) = half
            .link
            .routes
            .to_peer(half.link.ctl, candidate, Instant::now())?;
        half.server
            .send(Data::ToPeer(id, Box::new(msg)))
            .map_err(|_| CandidateError::Closed)
    }
}

impl CandidateReceiver {
    /// The device's next ICE candidate; `None` once the route is closed: no WebRTC session,
    /// [`WEB_ROUTE_TTL`](crate::WEB_ROUTE_TTL) passed, or the server stopped.
    pub async fn next(&mut self) -> Option<String> {
        let half = self.0.as_mut()?;
        if Instant::now() >= half.expires {
            return None;
        }
        if let Some(candidate) = half.pending.pop_front() {
            return Some(candidate);
        }
        loop {
            let msg = tokio::select! {
                msg = timeout_at(half.expires, half.rx.recv()) => msg.ok()??,
                () = half.server.closed() => return None,
            };
            if let Some(rendezvous_message::Union::IceCandidate(ice)) = msg.union {
                return Some(ice.candidate);
            }
        }
    }
}

/// The parts of the device's RelayResponse a web route keeps.
struct Answer {
    uuid: String,
    signed_pk: Vec<u8>,
    webrtc: String,
}

impl RendezvousServer {
    // The route table's lock is never held across an await, nor with another lock.
    pub(crate) async fn web_punch(&self, req: WebPunch) -> Result<WebRoute, WebPunchError> {
        let offer = req
            .offer
            .filter(|_| !self.switches.always_use_relay.load(Ordering::SeqCst));
        let session_key = match &offer {
            Some(offer) => Some(offer_session_key(offer).ok_or(WebPunchError::InvalidOffer)?),
            None => None,
        };
        let peer = self
            .pm
            .get(&req.target)
            .await
            .ok_or(WebPunchError::NotFound)?;
        let (elapsed, peer_addr, has_pk) = {
            let r = peer.read().await;
            (
                r.last_reg_time.elapsed().as_millis(),
                r.socket_addr,
                r.pk.len() == DEVICE_PK_LEN,
            )
        };
        if !has_pk {
            return Err(WebPunchError::NotFound);
        }
        if elapsed >= REG_TIMEOUT as u128 {
            return Err(WebPunchError::Offline);
        }
        // The configured first entry, not the health-checked list: the browser only reaches
        // the relay behind `/ws/relay`, and the check reorders by probe completion.
        let relay_server = self
            .relay_config
            .first()
            .cloned()
            .ok_or(WebPunchError::NoRelay)?;
        let claim = self
            .ws_addrs
            .claim(req.client.to_canonical())
            .ok_or(WebPunchError::Busy)?;
        let ctl = try_into_v4(claim.addr());
        let mangled = AddrMangle::encode(claim.addr());
        let (tx, mut rx) = mpsc::channel(LOCAL_SINK_LEN);
        let now = Instant::now();
        let expires = now + crate::WEB_ROUTE_TTL;
        let route = PunchRoute {
            peer_id: req.target.clone(),
            peer_ip: peer_addr.ip(),
            session_key: session_key.clone(),
            to_peer: 0,
            to_ctl: 0,
            expires,
            answered: false,
            to_ctl_tx: tx,
        };
        if !self.web_routes.open(ctl, route, now) {
            return Err(WebPunchError::Busy);
        }
        let link = RouteLink {
            routes: self.web_routes.clone(),
            ctl,
            _claim: claim,
        };
        let mut msg = RendezvousMessage::new();
        msg.set_punch_hole(PunchHole {
            socket_addr: mangled.into(),
            nat_type: NatType::SYMMETRIC.into(),
            relay_server: relay_server.clone(),
            webrtc_sdp_offer: offer.unwrap_or_default(),
            ..Default::default()
        });
        let mut pending = VecDeque::new();
        let res = match self.send_to_peer(&req.target, msg, peer_addr).await {
            Ok(()) => {
                let deadline = now + WEB_PUNCH_TIMEOUT;
                wait_answer(&mut rx, deadline, &relay_server, &mut pending).await
            }
            Err(_) => Err(WebPunchError::Stopped),
        };
        let answer = res?;
        let webrtc = Some(answer.webrtc).filter(|a| session_key.is_some() && !a.is_empty());
        let (sender, receiver) = match webrtc {
            Some(_) => {
                let link = Arc::new(link);
                let send = SendHalf {
                    link: link.clone(),
                    server: self.tx.clone(),
                };
                let recv = RecvHalf {
                    _link: link,
                    rx,
                    pending,
                    expires,
                    server: self.tx.clone(),
                };
                (CandidateSender(Some(send)), CandidateReceiver(Some(recv)))
            }
            // No WebRTC session: `link` is not kept, so the route goes when this returns.
            None => (CandidateSender(None), CandidateReceiver(None)),
        };
        Ok(WebRoute {
            uuid: answer.uuid,
            relay_server,
            signed_pk: answer.signed_pk,
            answer: webrtc,
            sender,
            receiver,
        })
    }

    /// Delivers a route's candidate to device `id` at its current address.
    pub(crate) async fn deliver_to_peer(&self, id: &str, msg: RendezvousMessage) {
        let Some(peer) = self.pm.get_in_memory(id).await else {
            return;
        };
        let peer_addr = peer.read().await.socket_addr;
        // Fails only once the server loop is gone.
        let _ = self.send_to_peer(id, msg, peer_addr).await;
    }
}

/// The device's RelayResponse on the punch's sink; candidates that come first are kept.
async fn wait_answer(
    rx: &mut mpsc::Receiver<RendezvousMessage>,
    deadline: Instant,
    relay_server: &str,
    pending: &mut VecDeque<String>,
) -> Result<Answer, WebPunchError> {
    loop {
        let msg = timeout_at(deadline, rx.recv())
            .await
            .ok()
            .flatten()
            .ok_or(WebPunchError::NoAnswer)?;
        match msg.union {
            Some(rendezvous_message::Union::IceCandidate(ice)) => pending.push_back(ice.candidate),
            Some(rendezvous_message::Union::RelayResponse(rr)) => {
                return answer_of(rr, relay_server);
            }
            _ => {}
        }
    }
}

fn answer_of(rr: RelayResponse, relay_server: &str) -> Result<Answer, WebPunchError> {
    if rr.relay_server != relay_server {
        return Err(WebPunchError::ForeignRelay {
            relay_server: rr.relay_server,
        });
    }
    let signed_pk = rr.pk().to_vec();
    if rr.uuid.is_empty() || signed_pk.is_empty() {
        return Err(WebPunchError::NoAnswer);
    }
    Ok(Answer {
        uuid: rr.uuid,
        signed_pk,
        webrtc: rr.webrtc_sdp_answer,
    })
}
