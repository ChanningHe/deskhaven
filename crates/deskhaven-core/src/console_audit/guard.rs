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
//!
//! The same holds for the actor: a guard built with [`TxGuard::for_account`] re-reads the
//! acting account's role and status in the transaction (share-locked on PostgreSQL, so a
//! concurrent demotion waits for this write or this write sees it) and refuses with
//! [`ACTOR_INACTIVE`] when the account is gone or no longer active. The decision then gets
//! the committed role, not the one the session carried when the request started.

use deskhaven_db::entity::user;
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, QuerySelect};

use super::{ConsoleAuditLog, NewConsoleAudit};

use deskhaven_db::entity::user::Role;

/// [`Refused::code`] when the acting account no longer exists or is no longer active.
pub const ACTOR_INACTIVE: &str = "actor_inactive";

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

/// The decision; gets the acting account's committed role when the guard has one.
type Decide<T> = Box<
    dyn FnOnce(Option<Role>, Option<&T>, Option<&T>) -> Result<NewConsoleAudit, Refused> + Send,
>;

struct Inner<T> {
    log: ConsoleAuditLog,
    /// Id of the acting account, re-read in the transaction ([`TxGuard::for_account`]).
    actor: Option<i64>,
    decide: Decide<T>,
}

/// Decides whether one write to a `T` may proceed and records its audit row (see the
/// module docs). `before` is `None` for a creation, `after` `None` for a deletion.
pub struct TxGuard<T> {
    inner: Option<Inner<T>>,
}

impl<T> TxGuard<T> {
    /// A guard that runs `decide` in the write transaction and records the row it returns
    /// through `log`. For actors that are not accounts (the CLI); console requests use
    /// [`for_account`](Self::for_account).
    pub fn new(
        log: &ConsoleAuditLog,
        decide: impl FnOnce(Option<&T>, Option<&T>) -> Result<NewConsoleAudit, Refused> + Send + 'static,
    ) -> Self {
        Self {
            inner: Some(Inner {
                log: log.clone(),
                actor: None,
                decide: Box::new(move |_, before, after| decide(before, after)),
            }),
        }
    }

    /// A guard acting as account `actor_id`: in the transaction it re-reads that account,
    /// refuses with [`ACTOR_INACTIVE`] unless it exists and is active, and passes its
    /// committed role to `decide`.
    pub fn for_account(
        log: &ConsoleAuditLog,
        actor_id: i64,
        decide: impl FnOnce(Role, Option<&T>, Option<&T>) -> Result<NewConsoleAudit, Refused>
        + Send
        + 'static,
    ) -> Self {
        Self {
            inner: Some(Inner {
                log: log.clone(),
                actor: Some(actor_id),
                decide: Box::new(move |role, before, after| {
                    decide(role.ok_or(Refused::new(ACTOR_INACTIVE))?, before, after)
                }),
            }),
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
        let Some(Inner { log, actor, decide }) = self.inner else {
            return Ok(());
        };
        let role = match actor {
            Some(id) => Some(
                active_role(conn, id)
                    .await?
                    .ok_or(Refused::new(ACTOR_INACTIVE))?,
            ),
            None => None,
        };
        let entry = decide(role, before, after)?;
        log.record_in(conn, &entry).await?;
        Ok(())
    }
}

/// The committed role of account `id` when it is active, share-locked on PostgreSQL
/// (SQLite's write transaction already excludes other writers).
async fn active_role<C: ConnectionTrait>(conn: &C, id: i64) -> Result<Option<Role>, DbErr> {
    let row = user::Entity::find_by_id(id)
        .select_only()
        .columns([user::Column::Role, user::Column::Status])
        .lock_shared()
        .into_tuple::<(Role, user::Status)>()
        .one(conn)
        .await?;
    Ok(row.and_then(|(role, status)| (status == user::Status::Active).then_some(role)))
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
