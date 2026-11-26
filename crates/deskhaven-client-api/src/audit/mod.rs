//! No Authorization: the device is identified by `id` + `uuid`. Every reply but 200 carries a
//! body, so no failure reads as "stored".

mod parse;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::post;
use deskhaven_core::audit::{Envelope, Ingest, Record};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::state::AppState;

use crate::ClientApiError;
use crate::client::Client;
use parse::ConnMessage;

/// A file record lists at most ten names next to its path.
pub const MAX_AUDIT_BODY: usize = 64 * 1024;
/// Same text as the heartbeat's.
const ID_NOT_FOUND: &str = "ID_NOT_FOUND";

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/audit/conn", post(conn))
        .route("/api/audit/file", post(file))
        .route("/api/audit/alarm", post(alarm))
        .layer(DefaultBodyLimit::max(MAX_AUDIT_BODY))
}

async fn conn(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    body: Bytes,
) -> Result<(), ClientApiError> {
    admit(&state, client)?;
    match parse::conn(&body)? {
        ConnMessage::Record(envelope, report) => {
            ingest(&state, envelope, Record::Conn(report)).await
        }
        ConnMessage::Note(note) => {
            // Every outcome gets the same reply: the note carries no uuid, so its reply must
            // not tell which ids are registered (not even through "no room", which only a
            // registered id can reach). The client ignores the reply anyway.
            let outcome = state.audit.record_note(note, client).await;
            let outcome = outcome.map_err(ClientApiError::internal)?;
            tracing::debug!(?outcome, "controller note");
            Ok(())
        }
    }
}

async fn file(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    body: Bytes,
) -> Result<(), ClientApiError> {
    admit(&state, client)?;
    let (envelope, report) = parse::file(&body)?;
    ingest(&state, envelope, Record::File(report)).await
}

async fn alarm(
    State(state): State<Arc<AppState>>,
    Client(client): Client,
    body: Bytes,
) -> Result<(), ClientApiError> {
    admit(&state, client)?;
    let (envelope, report) = parse::alarm(&body)?;
    ingest(&state, envelope, Record::Alarm(report)).await
}

fn admit(state: &AppState, client: ClientAddr) -> Result<(), ClientApiError> {
    state
        .audit
        .admit(client)
        .map_err(|retry_after| ClientApiError::TooManyRequests { retry_after })
}

/// A duplicate is answered like success: the client stops retrying a record it stored.
async fn ingest(
    state: &AppState,
    envelope: Envelope,
    record: Record,
) -> Result<(), ClientApiError> {
    let outcome = state.audit.ingest(envelope, record).await;
    match outcome.map_err(ClientApiError::internal)? {
        Ingest::Stored | Ingest::Duplicate => Ok(()),
        Ingest::NotFound => Err(not_found()),
        Ingest::TooFrequent { retry_after } => Err(ClientApiError::TooManyRequests { retry_after }),
    }
}

fn not_found() -> ClientApiError {
    ClientApiError::NotFound(ID_NOT_FOUND.into())
}
