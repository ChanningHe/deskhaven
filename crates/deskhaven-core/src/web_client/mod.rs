//! Authorization: an active device the user owns; administrators and owners any active device.
//! Anything else is not found. Sessions are in memory, keyed by an unguessable id, and answer only
//! their owner.

mod limits;
mod port;
mod relay;
mod service;
mod table;

pub use limits::{WEB_SESSIONS_PER_DEVICE_PER_MINUTE, WEB_SESSIONS_PER_USER_PER_MINUTE};
pub use port::{
    CandidateRejected, CandidateSink, CandidateSource, NextCandidate, PunchError, PunchFuture,
    PunchRequest, Punched, WebPunches,
};
pub use relay::{RELAY_PATH, relay_url, ws_origin};
pub use service::{IceServer, OpenRequest, Opened, Opener, WebClientError, WebClientService};
pub use table::{Candidates, MAX_SESSIONS_PER_USER, SESSION_TTL};
