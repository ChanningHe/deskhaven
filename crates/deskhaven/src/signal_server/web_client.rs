//! The core's web client port ([`WebPunches`]) over the current signal server: each punch
//! goes to the server serving at that moment (`SignalHandle::web_punch`); a route dies with
//! the server that made it.

use deskhaven_core::web_client::{
    CandidateRejected, CandidateSink, CandidateSource, NextCandidate, PunchError, PunchFuture,
    PunchRequest, Punched, WebPunches,
};
use deskhaven_signal::{
    CandidateError, CandidateReceiver, CandidateSender, WebPunch, WebPunchError,
};

use super::SignalStatus;

impl WebPunches for SignalStatus {
    fn punch(&self, req: PunchRequest) -> PunchFuture<'_> {
        let handle = self.current();
        Box::pin(async move {
            let punch = WebPunch {
                target: req.device_id,
                offer: req.offer,
                client: req.client,
            };
            let route = handle.web_punch(punch).await.map_err(punch_error)?;
            let (uuid, relay_server) = (route.uuid.clone(), route.relay_server.clone());
            let (signed_pk, answer) = (route.signed_pk.clone(), route.answer.clone());
            let (sender, receiver) = route.split();
            Ok(Punched {
                uuid,
                relay_server,
                signed_pk,
                answer,
                sender: Box::new(Sender(sender)),
                receiver: Box::new(Receiver(receiver)),
            })
        })
    }
}

fn punch_error(err: WebPunchError) -> PunchError {
    match err {
        WebPunchError::NotFound => PunchError::NotFound,
        WebPunchError::Offline => PunchError::Offline,
        WebPunchError::NoAnswer => PunchError::NoAnswer,
        WebPunchError::Busy => PunchError::Busy,
        WebPunchError::Stopped => PunchError::Stopped,
        WebPunchError::ForeignRelay { relay_server } => PunchError::ForeignRelay { relay_server },
        WebPunchError::NoRelay => PunchError::NoRelay,
        WebPunchError::InvalidOffer => PunchError::InvalidOffer,
    }
}

struct Sender(CandidateSender);

impl CandidateSink for Sender {
    fn send(&self, candidate: String) -> Result<(), CandidateRejected> {
        self.0.send(candidate).map_err(|err| match err {
            CandidateError::Closed => CandidateRejected::Closed,
            CandidateError::Invalid => CandidateRejected::Invalid,
            CandidateError::OverBudget => CandidateRejected::OverBudget,
        })
    }
}

struct Receiver(CandidateReceiver);

impl CandidateSource for Receiver {
    fn next(&mut self) -> NextCandidate<'_> {
        Box::pin(self.0.next())
    }
}
