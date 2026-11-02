//! [`TxGuard`]: authorization and console audit of one service write, decided inside the
//! write's own transaction.
//!
//! The API layer knows who acts and which rules apply (06 §4.1: an admin may not touch an
//! owner); the service knows the target's state, locked, in its transaction. A guard joins
//! the two: the service calls [`TxGuard::check`] with the target before and after the change,
//! still inside the transaction; the guard's closure either refuses ([`Refused`], the
//! service rolls back and returns it) or returns the audit row, which is inserted on the same
//! transaction. So the rule is evaluated against the committed state (no check-then-act
//! race) and the change commits together with its audit row, or neither does.

use sea_orm::{ConnectionTrait, DbErr};

use super::{ConsoleAuditLog, NewConsoleAudit};

/// A guard declined the write. `code` is a stable machine reason chosen by whoever built
/// the guard (the API layer maps it to its error codes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("refused: {code}")]
pub struct Refused {
    pub code: &'static str,
}

impl Refused {
    pub const fn new(code: &'static str) -> Self {
        Self { code }
    }
}

/// Why [`TxGuard::check`] failed.
#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error("console audit write failed")]
    Database(#[from] DbErr),
}

type Decide<T> = Box<dyn FnOnce(Option<&T>, Option<&T>) -> Result<NewConsoleAudit, Refused> + Send>;

/// Decides whether one write to a `T` may proceed and records its audit row (see the
/// module docs). `before` is `None` for a creation, `after` `None` for a deletion.
pub struct TxGuard<T> {
    inner: Option<(ConsoleAuditLog, Decide<T>)>,
}

impl<T> TxGuard<T> {
    /// A guard that runs `decide` in the write transaction and records the row it returns
    /// through `log`.
    pub fn new(
        log: &ConsoleAuditLog,
        decide: impl FnOnce(Option<&T>, Option<&T>) -> Result<NewConsoleAudit, Refused> + Send + 'static,
    ) -> Self {
        Self {
            inner: Some((log.clone(), Box::new(decide))),
        }
    }

    /// No rule and no row: the services' own unguarded entry points (login, provisioning,
    /// seeding), which are not console actions.
    pub(crate) fn none() -> Self {
        Self { inner: None }
    }

    /// Runs the decision on `conn` (the write transaction): refuses, or inserts the row.
    pub async fn check<C: ConnectionTrait>(
        self,
        conn: &C,
        before: Option<&T>,
        after: Option<&T>,
    ) -> Result<(), GuardError> {
        let Some((log, decide)) = self.inner else {
            return Ok(());
        };
        let entry = decide(before, after)?;
        log.record_in(conn, &entry).await?;
        Ok(())
    }
}

impl<T> std::fmt::Debug for TxGuard<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = if self.inner.is_some() {
            "TxGuard(audited)"
        } else {
            "TxGuard(none)"
        };
        f.write_str(kind)
    }
}
