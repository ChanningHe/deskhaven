//! `foreign_relay` names the device's relay (configuration, not a secret) so the console can
//! say what to change.

use std::time::Duration;

use deskhaven_core::web_client::{CandidateRejected, PunchError, WebClientError};
use serde_json::json;

use crate::{AdminApiError, ErrorCode};

const BUSY_RETRY_AFTER: Duration = Duration::from_secs(1);

impl From<WebClientError> for AdminApiError {
    fn from(err: WebClientError) -> Self {
        match err {
            WebClientError::Disabled => Self::new(
                ErrorCode::WebClientDisabled,
                "the web client is disabled (web_client.enabled)",
            ),
            WebClientError::Unavailable => Self::unavailable(BUSY_RETRY_AFTER),
            WebClientError::NotFound => Self::not_found("no such device or session"),
            WebClientError::RateLimited(wait) => Self::rate_limited(wait),
            WebClientError::TooManySessions => Self::new(
                ErrorCode::TooManySessions,
                "too many open web client sessions; close one first",
            ),
            WebClientError::Punch(err) => punch(err),
            WebClientError::Candidate(err) => candidate(err),
            WebClientError::Closed => closed(),
            err @ (WebClientError::Database(_) | WebClientError::Crypto(_)) => Self::internal(err),
        }
    }
}

fn punch(err: PunchError) -> AdminApiError {
    match err {
        PunchError::NotFound => AdminApiError::not_found("no such device or session"),
        PunchError::Offline => {
            AdminApiError::new(ErrorCode::DeviceOffline, "the device is offline")
        }
        PunchError::NoAnswer => AdminApiError::new(
            ErrorCode::DeviceNoAnswer,
            "the device did not answer in time",
        ),
        PunchError::Busy => AdminApiError::rate_limited(BUSY_RETRY_AFTER),
        PunchError::Stopped => AdminApiError::unavailable(BUSY_RETRY_AFTER),
        PunchError::ForeignRelay { relay_server } => AdminApiError::new(
            ErrorCode::ForeignRelay,
            "the device uses a relay server of its own",
        )
        .with_details(json!({ "relay_server": relay_server })),
        PunchError::NoRelay => {
            AdminApiError::new(ErrorCode::NoRelay, "no relay server is configured")
        }
        PunchError::InvalidOffer => {
            AdminApiError::new(ErrorCode::InvalidOffer, "invalid WebRTC offer")
        }
    }
}

fn candidate(err: CandidateRejected) -> AdminApiError {
    match err {
        CandidateRejected::Closed => closed(),
        CandidateRejected::Invalid => {
            AdminApiError::new(ErrorCode::InvalidCandidate, "invalid ICE candidate")
        }
        CandidateRejected::OverBudget => AdminApiError::new(
            ErrorCode::TooManyCandidates,
            "too many ICE candidates for this session",
        ),
    }
}

fn closed() -> AdminApiError {
    AdminApiError::new(ErrorCode::SessionClosed, "the web client session is closed")
}
