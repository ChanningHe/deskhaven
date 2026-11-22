//! The signal server's side of a web client session, as the core sees it: `serve`
//! implements [`WebPunches`] over the running signal server (`SignalHandle::web_punch`); the
//! core never depends on the signal crate.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

/// A punch on behalf of a browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PunchRequest {
    /// The device's RustDesk id.
    pub device_id: String,
    /// The browser's WebRTC offer endpoint; `None` for relay only.
    pub offer: Option<String>,
    /// The browser's IP as the HTTP stack resolved it (trusted proxies applied). The device
    /// sees it as the controller's address (its IP whitelist applies to it).
    pub client: IpAddr,
}

/// An answered punch. Dropping both candidate halves revokes its route.
pub struct Punched {
    /// The relay session's uuid.
    pub uuid: String,
    /// The relay server the device connects to.
    pub relay_server: String,
    /// The device key as the server signed it (`IdPk`), for the browser to verify.
    pub signed_pk: Vec<u8>,
    /// The device's WebRTC answer, if an offer went out and it answered one.
    pub answer: Option<String>,
    pub sender: Box<dyn CandidateSink>,
    pub receiver: Box<dyn CandidateSource>,
}

impl std::fmt::Debug for Punched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Punched")
            .field("uuid", &self.uuid)
            .field("relay_server", &self.relay_server)
            .field("answer", &self.answer.is_some())
            .finish_non_exhaustive()
    }
}

/// Why a punch failed (the signal server's `WebPunchError`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PunchError {
    #[error("device not registered with the signal server")]
    NotFound,
    #[error("device is offline")]
    Offline,
    #[error("the device did not answer")]
    NoAnswer,
    #[error("too many web punches in flight")]
    Busy,
    #[error("signal server is not running")]
    Stopped,
    /// The device relays through a relay server of its own, which the browser cannot reach.
    #[error("the device uses its own relay server")]
    ForeignRelay { relay_server: String },
    #[error("no relay server is configured")]
    NoRelay,
    #[error("invalid WebRTC offer")]
    InvalidOffer,
}

/// Why a browser candidate was refused (the signal server's `CandidateError`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CandidateRejected {
    /// No WebRTC session, the route expired, or the server stopped.
    #[error("the route is closed")]
    Closed,
    #[error("invalid ICE candidate")]
    Invalid,
    #[error("too many ICE candidates for this route")]
    OverBudget,
}

/// The browser's half of a route's trickle ICE: towards the device.
pub trait CandidateSink: Send + Sync {
    fn send(&self, candidate: String) -> Result<(), CandidateRejected>;
}

/// A pending [`CandidateSource::next`].
pub type NextCandidate<'a> = Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>>;

/// The device's half of a route's trickle ICE: towards the browser.
pub trait CandidateSource: Send {
    /// The device's next candidate; `None` once the route is closed. Cancel-safe: dropping
    /// the future loses no candidate.
    fn next(&mut self) -> NextCandidate<'_>;
}

/// A pending [`WebPunches::punch`].
pub type PunchFuture<'a> = Pin<Box<dyn Future<Output = Result<Punched, PunchError>> + Send + 'a>>;

/// The signal server in this process.
pub trait WebPunches: Send + Sync {
    /// Asks the device for a relay session (and with an offer a WebRTC answer).
    fn punch(&self, req: PunchRequest) -> PunchFuture<'_>;
}
