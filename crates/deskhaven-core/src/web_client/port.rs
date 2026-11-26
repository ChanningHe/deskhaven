//! The core never depends on the signal crate.

use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PunchRequest {
    pub device_id: String,
    pub offer: Option<String>,
    /// The device sees it as the controller's address (its IP whitelist applies to it).
    pub client: IpAddr,
}

/// Dropping both candidate halves revokes its route.
pub struct Punched {
    pub uuid: String,
    pub relay_server: String,
    /// The device key as the server signed it, for the browser to verify.
    pub signed_pk: Vec<u8>,
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
    #[error("the device uses its own relay server")]
    ForeignRelay { relay_server: String },
    #[error("no relay server is configured")]
    NoRelay,
    #[error("invalid WebRTC offer")]
    InvalidOffer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CandidateRejected {
    #[error("the route is closed")]
    Closed,
    #[error("invalid ICE candidate")]
    Invalid,
    #[error("too many ICE candidates for this route")]
    OverBudget,
}

pub trait CandidateSink: Send + Sync {
    fn send(&self, candidate: String) -> Result<(), CandidateRejected>;
}

pub type NextCandidate<'a> = Pin<Box<dyn Future<Output = Option<String>> + Send + 'a>>;

pub trait CandidateSource: Send {
    /// Cancel-safe: dropping the future loses no candidate.
    fn next(&mut self) -> NextCandidate<'_>;
}

pub type PunchFuture<'a> = Pin<Box<dyn Future<Output = Result<Punched, PunchError>> + Send + 'a>>;

pub trait WebPunches: Send + Sync {
    fn punch(&self, req: PunchRequest) -> PunchFuture<'_>;
}
