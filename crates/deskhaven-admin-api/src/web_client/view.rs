use deskhaven_core::web_client::IceServer;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// The signal server's limit.
pub const MAX_OFFER_BYTES: usize = 4096;
/// The signal server's limit.
pub const MAX_CANDIDATE_BYTES: usize = 2048;
/// Below the HTTP request timeout.
pub const MAX_POLL_WAIT_SECS: u32 = 20;
const MAX_DEVICE_ID_CHARS: usize = 64;
const MAX_AFTER: u32 = 1024;

/// Opens a web client session to a device.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenWebClientSession {
    /// The RustDesk id of the device.
    #[garde(length(chars, min = 1, max = MAX_DEVICE_ID_CHARS), custom(rustdesk_id_chars))]
    pub device_id: String,
    /// The browser's WebRTC offer endpoint (`webrtc://` + base64 of the JSON session
    /// description); without it only the relay is prepared.
    #[garde(length(bytes, min = 1, max = MAX_OFFER_BYTES))]
    pub offer: Option<String>,
}

fn rustdesk_id_chars(value: &str, _context: &()) -> garde::Result {
    let valid = value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    match valid {
        true => Ok(()),
        false => Err(garde::Error::new("must be letters, digits, - or _")),
    }
}

/// One ICE server for `RTCPeerConnection` (`RTCIceServer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct IceServerView {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

impl From<IceServer> for IceServerView {
    fn from(server: IceServer) -> Self {
        Self {
            urls: server.urls,
            username: server.username,
            credential: server.credential,
        }
    }
}

/// An opened web client session: everything the browser needs to reach the device over
/// the relay, or over WebRTC when `answer` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct WebClientSession {
    /// For the candidate and close endpoints; valid for the user who opened it only.
    pub session_id: String,
    /// The device's RustDesk id (`LoginRequest.username`).
    pub peer_id: String,
    /// Base64 of the device key as the signal server signed it (`IdPk`); verify it with
    /// `server_pk` before trusting the device's `SignedId`.
    pub signed_peer_pk: String,
    /// The signal server's public key (base64), the one RustDesk clients are configured with.
    pub server_pk: String,
    /// The relay session's uuid (`RequestRelay.uuid`).
    pub uuid: String,
    /// The relay's WebSocket URL; a bare path is on the console's own origin.
    pub relay_url: String,
    /// The device's WebRTC answer endpoint, when an offer went out and it answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    pub ice_servers: Vec<IceServerView>,
    /// `LoginRequest.my_id`: the console user name, chosen by the server.
    pub my_id: String,
    /// `LoginRequest.my_name`: the user's display name, else the user name.
    pub my_name: String,
    /// `signal.require_kx_v1`: a relayed session must not fall back to key exchange v0.
    pub require_kx_v1: bool,
}

/// One of the browser's ICE candidates.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WebClientCandidate {
    /// The candidate line as `RTCIceCandidate.candidate` gives it.
    #[garde(length(bytes, min = 1, max = MAX_CANDIDATE_BYTES))]
    pub candidate: String,
}

/// Which device candidates to return, and how long to wait for one.
#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct CandidatesQuery {
    /// Index of the first candidate wanted (the `next` of the previous page); default 0.
    #[garde(range(max = MAX_AFTER))]
    pub after: Option<u32>,
    /// Seconds to wait when there is none yet (0-20); default 20.
    #[garde(range(max = MAX_POLL_WAIT_SECS))]
    pub wait: Option<u32>,
}

/// The device's ICE candidates from `after` on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct WebClientCandidates {
    pub candidates: Vec<String>,
    /// The `after` of the next poll.
    pub next: u32,
}
