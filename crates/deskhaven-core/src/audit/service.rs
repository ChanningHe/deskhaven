//! [`AuditService`]: identity check, rate limits, nonce claim and the record write, in one
//! transaction.

use std::sync::Arc;
use std::time::Duration;

use deskhaven_db::begin_write;
use deskhaven_db::entity::{audit_nonce, connection_session};
use deskhaven_db::time::TimestampMs;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbErr, EntityTrait,
    PaginatorTrait, QueryFilter, TryInsertResult,
};

use super::limits::{AUDIT_PER_IP_PER_MINUTE, DeviceLimit};
use super::notes::{Hold, PendingNotes};
use super::{AuditError, ControllerNote, Envelope, Ingest, NoteOutcome, Record, SessionId};
use super::{conn, records};
use crate::auth::limiter::{ClientAddr, IpLimiter};
use crate::clock::SharedClock;
use crate::devices::DeviceService;
use crate::text::{self, NOTE_MAX_LEN};

/// Audit ingest. Cheap to clone (shared state).
#[derive(Clone)]
pub struct AuditService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    devices: DeviceService,
    clock: SharedClock,
    notes: PendingNotes,
    ips: IpLimiter,
    device_limit: DeviceLimit,
}

impl AuditService {
    /// `devices` verifies the reporting device; times (rows, rate limits, waiting notes)
    /// come from `clock`.
    pub fn new(db: DatabaseConnection, devices: DeviceService, clock: SharedClock) -> Self {
        Self {
            inner: Arc::new(Inner {
                db,
                devices,
                notes: PendingNotes::new(Arc::clone(&clock)),
                ips: IpLimiter::fixed(AUDIT_PER_IP_PER_MINUTE, Arc::clone(&clock)),
                device_limit: DeviceLimit::new(Arc::clone(&clock)),
                clock,
            }),
        }
    }

    /// Takes a token of `client`'s audit bucket, or returns how long until one is free.
    pub fn admit(&self, client: ClientAddr) -> Result<(), Duration> {
        self.inner.ips.check(client)
    }

    /// Stores `record` of the device `envelope` names, unless the device is not verified
    /// (nothing written, no nonce claimed, no quota spent), over its quota, or the nonce was
    /// stored before. The nonce is claimed in the record's transaction, so a failed write
    /// leaves it free for the retry. The write runs in its own task: a dropped request cannot
    /// cut it short.
    pub async fn ingest(&self, envelope: Envelope, record: Record) -> Result<Ingest, AuditError> {
        let inner = &self.inner;
        let verified = inner
            .devices
            .verify(&envelope.rustdesk_id, &envelope.uuid)
            .await?;
        if verified.is_none() {
            return Ok(Ingest::NotFound);
        }
        if let Err(retry_after) = inner.device_limit.check(&envelope.rustdesk_id) {
            return Ok(Ingest::TooFrequent { retry_after });
        }
        let inner = Arc::clone(inner);
        tokio::spawn(async move { inner.store(&envelope, &record).await })
            .await
            .map_err(|_| AuditError::Aborted)?
            .map_err(AuditError::from)
    }

    /// Sets the controller's note on its session's rows unless they have one, or keeps it
    /// until a record binds the session (see `notes`). The note carries no uuid: the id must
    /// be registered, and the session id (a random `u64` of the controller) selects the rows.
    /// `from` is charged for a waiting note.
    pub async fn record_note(
        &self,
        note: ControllerNote,
        from: ClientAddr,
    ) -> Result<NoteOutcome, AuditError> {
        let inner = &self.inner;
        if !inner.devices.is_registered(&note.rustdesk_id).await? {
            return Ok(NoteOutcome::NotFound);
        }
        let text = text::clean(&note.note, NOTE_MAX_LEN, true);
        if text.trim().is_empty() {
            return Ok(NoteOutcome::Ignored);
        }
        let (device, session) = (note.rustdesk_id.as_str(), &note.session_id);
        // Held before the update: a record binding the session meanwhile takes it after its
        // commit, so the note is never lost between the two.
        let hold = inner.notes.hold(device, session, text.clone(), from);
        if set_controller_note(&inner.db, device, session, &text).await? {
            inner.notes.take(device, session);
            return Ok(NoteOutcome::Stored);
        }
        if has_session(&inner.db, device, session).await? {
            inner.notes.take(device, session);
            return Ok(NoteOutcome::Ignored);
        }
        Ok(inner.pending(device, hold))
    }
}

impl Inner {
    async fn store(&self, envelope: &Envelope, record: &Record) -> Result<Ingest, DbErr> {
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

    /// The outcome of a note whose session has no rows yet.
    fn pending(&self, device: &str, hold: Hold) -> NoteOutcome {
        match hold {
            Hold::Held | Hold::AlreadyWaiting => NoteOutcome::Pending,
            Hold::SourceFull | Hold::DeviceFull => {
                tracing::debug!(rustdesk_id = device, ?hold, "controller note refused");
                NoteOutcome::Full
            }
            Hold::Full => {
                if self.notes.warn_due() {
                    tracing::warn!("controller notes dropped: too many waiting for sessions");
                }
                NoteOutcome::Full
            }
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

/// Sets `note` on the session's rows that have none (the first note stays, 06 §7); `true`
/// if any row took it.
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
        .filter(connection_session::Column::ControllerNote.is_null())
        .exec(db)
        .await?;
    Ok(updated.rows_affected > 0)
}

async fn has_session(
    db: &DatabaseConnection,
    device: &str,
    session: &SessionId,
) -> Result<bool, DbErr> {
    let rows = connection_session::Entity::find()
        .filter(connection_session::Column::DeviceRustdeskId.eq(device))
        .filter(connection_session::Column::SessionId.eq(session.as_str()))
        .count(db)
        .await?;
    Ok(rows > 0)
}
