//! The web client endpoints (see the module docs of `web_client`).

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use deskhaven_core::state::AppState;
use deskhaven_core::web_client::{OpenRequest, Opened, Opener, relay_url};
use serde::Serialize;
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::TryRecvError;
use tracing::Instrument;

use super::view::{
    CandidatesQuery, MAX_POLL_WAIT_SECS, OpenWebClientSession, WebClientCandidate,
    WebClientCandidates, WebClientSession,
};
use crate::audit::{Audit, AuditCtx, Audited};
use crate::session::ConsoleUser;
use crate::{AdminApiError, ValidJson, ValidPath, ValidQuery};

/// `result` of a request dropped before the punch ended.
const CANCELLED: &str = "cancelled";

/// What a session request's audit row records: never the offer, the answer or candidates.
#[derive(Serialize)]
struct OpenOutcome {
    /// Whether the browser sent a WebRTC offer.
    offer: bool,
    /// `ok`, or the error code answered.
    result: &'static str,
    /// Whether the device answered the offer (the browser can try WebRTC).
    #[serde(skip_serializing_if = "Option::is_none")]
    webrtc: Option<bool>,
}

/// Opens a web client session: the server asks the device for a relay session (and, with
/// an offer, a WebRTC answer) on the browser's behalf. Audited as `web_client.session.open`,
/// whatever the outcome.
#[utoipa::path(
    post,
    path = "/web-client/sessions",
    operation_id = "open_web_client_session",
    tag = "web-client",
    security(("console_session" = ["self"])),
    request_body = OpenWebClientSession,
    responses(
        (status = 201, description = "The session", body = WebClientSession),
        (status = 400, description = "The offer is invalid (`invalid_offer`)", body = crate::ErrorEnvelope),
        (status = 403, description = "The web client is disabled (`web_client_disabled`)", body = crate::ErrorEnvelope),
        (status = 404, description = "No such device the user may control (`not_found`)", body = crate::ErrorEnvelope),
        (status = 409, description = "The device is offline (`device_offline`), uses a relay of its own (`foreign_relay`, `details.relay_server`), or no relay is configured (`no_relay`)", body = crate::ErrorEnvelope),
        (status = 429, description = "Too many sessions per minute (`rate_limited`) or at once (`too_many_sessions`)", body = crate::ErrorEnvelope),
        (status = 503, description = "No signal server is running in this process (`unavailable`)", body = crate::ErrorEnvelope),
        (status = 504, description = "The device did not answer in time (`device_no_answer`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn open(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<OpenWebClientSession>,
) -> Result<(StatusCode, Audited, Json<WebClientSession>), AdminApiError> {
    let user = state.users.get(principal.user_id).await?;
    let by = Opener {
        user_id: principal.user_id,
        role: principal.role,
    };
    let req = OpenRequest {
        device_id: body.device_id.clone(),
        offer: body.offer,
        client: ctx.ip,
    };
    // The punch and its audit row run in their own task: a request dropped mid-punch (client
    // gone, deadline) still leaves its row, and the session it would have got is closed.
    let (request_alive, alive) = oneshot::channel::<()>();
    let task =
        tokio::spawn(open_audited(Arc::clone(&state), ctx, by, req, alive).in_current_span());
    let opened = task.await.map_err(AdminApiError::internal)??;
    drop(request_alive);
    let my_name = match user.display_name.trim() {
        "" => user.username.clone(),
        name => name.to_owned(),
    };
    let session = view(&state, body.device_id, opened, user.username, my_name);
    Ok((StatusCode::CREATED, Audited, Json(session)))
}

/// Opens the session and records `web_client.session.open`. `requester` closes once the
/// request is dropped: a session opened for nobody is closed again and recorded as
/// `cancelled`. A session whose row cannot be written is closed too.
async fn open_audited(
    state: Arc<AppState>,
    ctx: AuditCtx,
    by: Opener,
    req: OpenRequest,
    mut requester: oneshot::Receiver<()>,
) -> Result<Opened, AdminApiError> {
    let device_id = req.device_id.clone();
    let offer = req.offer.is_some();
    let mut result = state
        .web_client
        .open(by, req)
        .await
        .map_err(AdminApiError::from);
    let cancelled = matches!(requester.try_recv(), Err(TryRecvError::Closed));
    if let (true, Ok(opened)) = (cancelled, &result) {
        state.web_client.close(by.user_id, &opened.id);
    }
    let outcome = OpenOutcome {
        offer,
        result: match &result {
            _ if cancelled => CANCELLED,
            Ok(_) => "ok",
            Err(err) => err.code().as_str(),
        },
        webrtc: result.as_ref().ok().map(|o| o.answer.is_some()),
    };
    let audit = Audit::new("web_client.session.open")
        .target("device", &device_id)
        .after(&outcome);
    if let Err(err) = state.console_audit.record(&ctx.entry(audit)).await {
        if let Ok(opened) = &result {
            state.web_client.close(by.user_id, &opened.id);
        }
        result = Err(AdminApiError::internal(err));
    }
    result
}

fn view(
    state: &AppState,
    peer_id: String,
    opened: Opened,
    my_id: String,
    my_name: String,
) -> WebClientSession {
    WebClientSession {
        session_id: opened.id,
        peer_id,
        signed_peer_pk: STANDARD.encode(&opened.signed_pk),
        server_pk: state.signal_public_key.clone(),
        uuid: opened.uuid,
        relay_url: relay_url(&state.public_url),
        answer: opened.answer,
        ice_servers: state
            .web_client
            .ice_servers()
            .into_iter()
            .map(Into::into)
            .collect(),
        my_id,
        my_name,
    }
}

/// Passes one of the browser's ICE candidates to the device. Not audited (candidates are
/// network details of the session the open row covers).
#[utoipa::path(
    post,
    path = "/web-client/sessions/{id}/candidates",
    operation_id = "add_web_client_candidate",
    tag = "web-client",
    security(("console_session" = ["self"])),
    params(("id" = String, Path, description = "Session id")),
    request_body = WebClientCandidate,
    responses(
        (status = 204, description = "Passed to the device"),
        (status = 400, description = "The signal server refuses the candidate (`invalid_candidate`)", body = crate::ErrorEnvelope),
        (status = 404, description = "No such session of the user (`not_found`)", body = crate::ErrorEnvelope),
        (status = 410, description = "The session's route is closed (`session_closed`)", body = crate::ErrorEnvelope),
        (status = 429, description = "The session passed the most candidates (`too_many_candidates`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn add_candidate(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ValidPath(id): ValidPath<String>,
    ValidJson(body): ValidJson<WebClientCandidate>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    state
        .web_client
        .send_candidate(principal.user_id, &id, body.candidate)?;
    Ok((Audited, StatusCode::NO_CONTENT))
}

/// The device's ICE candidates from `after` on; when there are none yet, waits up to
/// `wait` seconds for one (long poll) and answers an empty page if none came.
#[utoipa::path(
    get,
    path = "/web-client/sessions/{id}/candidates",
    operation_id = "poll_web_client_candidates",
    tag = "web-client",
    security(("console_session" = ["self"])),
    params(("id" = String, Path, description = "Session id"), CandidatesQuery),
    responses(
        (status = 200, description = "The candidates and the next index", body = WebClientCandidates),
        (status = 404, description = "No such session of the user (`not_found`)", body = crate::ErrorEnvelope),
        (status = 410, description = "The route is closed and every candidate was read (`session_closed`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn poll_candidates(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ValidPath(id): ValidPath<String>,
    ValidQuery(query): ValidQuery<CandidatesQuery>,
) -> Result<Json<WebClientCandidates>, AdminApiError> {
    let after = query.after.unwrap_or(0);
    let wait = Duration::from_secs(u64::from(query.wait.unwrap_or(MAX_POLL_WAIT_SECS)));
    let page = state
        .web_client
        .candidates(principal.user_id, &id, after as usize, wait)
        .await?;
    Ok(Json(WebClientCandidates {
        candidates: page.candidates,
        next: u32::try_from(page.next).unwrap_or(u32::MAX),
    }))
}

/// Closes a web client session and revokes its route. Audited as
/// `web_client.session.close`.
#[utoipa::path(
    delete,
    path = "/web-client/sessions/{id}",
    operation_id = "close_web_client_session",
    tag = "web-client",
    security(("console_session" = ["self"])),
    params(("id" = String, Path, description = "Session id")),
    responses(
        (status = 204, description = "Closed"),
        (status = 404, description = "No such session of the user (`not_found`)", body = crate::ErrorEnvelope),
    )
)]
pub async fn close(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidPath(id): ValidPath<String>,
) -> Result<(Audited, StatusCode), AdminApiError> {
    let device_id = state
        .web_client
        .close(principal.user_id, &id)
        .ok_or_else(|| AdminApiError::not_found("no such device or session"))?;
    let audit = Audit::new("web_client.session.close").target("device", device_id);
    state
        .console_audit
        .record(&ctx.entry(audit))
        .await
        .map_err(AdminApiError::internal)?;
    Ok((Audited, StatusCode::NO_CONTENT))
}
