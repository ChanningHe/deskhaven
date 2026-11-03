//! [`AuditService`]: identity check, nonce claim and the record write, in one transaction.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_db::begin_write;
use deskhaven_db::entity::{audit_nonce, connection_session};
use deskhaven_db::time::TimestampMs;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    QueryFilter, SqlErr, TryInsertResult,
};

use super::notes::PendingNotes;
use super::{AuditError, ControllerNote, Envelope, Ingest, NoteOutcome, Record, SessionId};
use super::{conn, records};
use crate::auth::limiter::{ClientAddr, IpLimiter};
use crate::clock::SharedClock;
use crate::devices::{DeviceRepository, DeviceService};
use crate::text::{self, NOTE_MAX_LEN};

/// Audit requests per client network per minute (burst the same), every request charged:
/// each one writes. A device posts a handful per connection; a burst of alarms (failed
/// logins) or file records stays far below it, an office NAT included.
pub const AUDIT_PER_IP_PER_MINUTE: u32 = 6_000;
/// Two transactions racing to create one connection's row: the loser's insert hits the
/// unique key and it runs again, now merging into the winner's row.
const MAX_ATTEMPTS: usize = 2;

/// Audit ingest. Cheap to clone (shared state).
#[derive(Clone)]
pub struct AuditService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    devices: DeviceService,
    registry: DeviceRepository,
    clock: SharedClock,
    notes: PendingNotes,
    ips: IpLimiter,
}

impl AuditService {
    /// `devices` verifies the reporting device; times (rows, rate limit, waiting notes) come
    /// from `clock`.
    pub fn new(db: DatabaseConnection, devices: DeviceService, clock: SharedClock) -> Self {
        Self {
            inner: Arc::new(Inner {
                registry: DeviceRepository::new(db.clone()),
                db,
                devices,
                notes: PendingNotes::new(Arc::clone(&clock)),
                ips: IpLimiter::fixed(AUDIT_PER_IP_PER_MINUTE, Arc::clone(&clock)),
                clock,
            }),
        }
    }

    /// Takes a token of `client`'s audit bucket, or returns how long until one is free.
    pub fn admit(&self, client: ClientAddr) -> Result<(), Duration> {
        self.inner.ips.check(client)
    }

    /// Stores `record` of the device `envelope` names, unless the device is not verified
    /// (nothing written, no nonce claimed) or its nonce was stored before. The nonce is
    /// claimed in the record's transaction, so a failed write leaves it free for the retry.
    /// The write runs in its own task: a dropped request cannot cut it short.
    pub async fn ingest(&self, envelope: Envelope, record: Record) -> Result<Ingest, AuditError> {
        let verified = self
            .inner
            .devices
            .verify(&envelope.rustdesk_id, &envelope.uuid)
            .await?;
        if verified.is_none() {
            return Ok(Ingest::NotFound);
        }
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move { inner.store(&envelope, &record).await })
            .await
            .map_err(|_| AuditError::Aborted)?
    }

    /// Sets the controller's note of a session on its rows, or keeps it until a record binds
    /// the session (see `notes`). The note carries no uuid: the id must be registered, and
    /// the session id (a random `u64` of the controller) is what selects the rows.
    pub async fn record_note(&self, note: ControllerNote) -> Result<NoteOutcome, AuditError> {
        let inner = &self.inner;
        if inner
            .registry
            .find_by_rustdesk_id(&note.rustdesk_id)
            .await?
            .is_none()
        {
            return Ok(NoteOutcome::NotFound);
        }
        let text = text::clean(&note.note, NOTE_MAX_LEN, true);
        // Held before the update: a record binding the session meanwhile takes it after its
        // commit, so the note is never lost between the two.
        let held = inner
            .notes
            .hold(&note.rustdesk_id, &note.session_id, text.clone());
        let updated = set_controller_note(&inner.db, &note.rustdesk_id, &note.session_id, &text);
        if updated.await? {
            inner.notes.take(&note.rustdesk_id, &note.session_id);
            return Ok(NoteOutcome::Stored);
        }
        if !held {
            tracing::warn!(rustdesk_id = %note.rustdesk_id, "controller note dropped: too many waiting");
            return Ok(NoteOutcome::Full);
        }
        Ok(NoteOutcome::Pending)
    }
}

impl Inner {
    async fn store(&self, envelope: &Envelope, record: &Record) -> Result<Ingest, AuditError> {
        let mut attempt = 1;
        loop {
            match self.store_once(envelope, record).await {
                Err(err) if attempt < MAX_ATTEMPTS && is_unique_violation(&err) => {
                    tracing::debug!(error = %err, "audit write raced, retrying");
                    attempt += 1;
                }
                outcome => return outcome.map_err(AuditError::from),
            }
        }
    }

    async fn store_once(&self, envelope: &Envelope, record: &Record) -> Result<Ingest, DbErr> {
        let now = self.clock.now();
        let device = envelope.rustdesk_id.as_str();
        let tx = begin_write(&self.db).await?;
        if let Some(nonce) = &envelope.nonce
            && !claim(&tx, nonce, now).await?
        {
            tx.rollback().await?;
            tracing::debug!(rustdesk_id = device, "audit record already stored");
            return Ok(Ingest::Duplicate);
        }
        match record {
            Record::Conn(report) => conn::merge(&tx, device, report, now).await?,
            Record::File(report) => records::insert_file(&tx, device, report, now).await?,
            Record::Alarm(report) => records::insert_alarm(&tx, device, report, now).await?,
        }
        tx.commit().await?;
        if let Record::Conn(report) = record {
            self.apply_waiting_note(device, &report.session_id).await;
        }
        Ok(Ingest::Stored)
    }

    /// Sets a note that waited for `session` now that a record bound it. The record is
    /// committed, so a failure here costs only the note.
    async fn apply_waiting_note(&self, device: &str, session: &SessionId) {
        if session.is_unbound() {
            return;
        }
        let Some(note) = self.notes.take(device, session) else {
            return;
        };
        if let Err(err) = set_controller_note(&self.db, device, session, &note).await {
            tracing::error!(rustdesk_id = device, error = %err, "controller note lost");
        }
    }
}

/// Records `nonce`; `false` if it was already stored.
async fn claim(tx: &DatabaseTransaction, nonce: &str, now: TimestampMs) -> Result<bool, DbErr> {
    let row = audit_nonce::ActiveModel {
        nonce: Set(nonce.to_owned()),
        received_at: Set(now),
    };
    let inserted = audit_nonce::Entity::insert(row)
        .on_conflict_do_nothing_on([audit_nonce::Column::Nonce])
        .exec_without_returning(tx)
        .await?;
    Ok(matches!(inserted, TryInsertResult::Inserted(n) if n > 0))
}

/// `true` if the session had rows.
async fn set_controller_note(
    db: &impl ConnectionTrait,
    device: &str,
    session: &SessionId,
    note: &str,
) -> Result<bool, DbErr> {
    let updated = connection_session::Entity::update_many()
        .col_expr(
            connection_session::Column::ControllerNote,
            Expr::value(note.to_owned()),
        )
        .filter(connection_session::Column::DeviceRustdeskId.eq(device))
        .filter(connection_session::Column::SessionId.eq(session.as_str()))
        .exec(db)
        .await?;
    Ok(updated.rows_affected > 0)
}

fn is_unique_violation(err: &DbErr) -> bool {
    matches!(err.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}
