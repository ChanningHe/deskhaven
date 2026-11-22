//! Web client sessions (remote control from the console, `web_client.*` settings): the
//! server punches the device on the browser's behalf through the signal server in this
//! process ([`WebPunches`], installed by `serve`), keeps the resulting session for the user
//! who opened it, and relays the WebRTC session's ICE candidates both ways.
//!
//! - Off unless `web_client.enabled`.
//! - Authorization: an active device the user owns; administrators and owners any active
//!   device. Anything else is not found.
//! - Bounds: [`WEB_SESSIONS_PER_USER_PER_MINUTE`], [`WEB_SESSIONS_PER_DEVICE_PER_MINUTE`],
//!   [`MAX_SESSIONS_PER_USER`] at once, each for [`SESSION_TTL`]; the signal server bounds the
//!   offer, the candidates and the punches in flight.
//! - Sessions are in memory, per process, keyed by an unguessable id, and answer only their
//!   owner.

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
