//! The rule is evaluated inside the write's own transaction against the committed state (no
//! check-then-act race), and the change commits together with its audit row or neither does.
//! [`TxGuard::for_account`] re-reads the acting account there, so a concurrent demotion waits
//! for this write or this write sees it.

use deskhaven_db::entity::user;
use sea_orm::{ConnectionTrait, DbErr, EntityTrait, QuerySelect};

use super::{ConsoleAuditLog, NewConsoleAudit};

use deskhaven_db::entity::user::Role;

pub const ACTOR_INACTIVE: &str = "actor_inactive";

/// `code` is a stable machine reason; the API layer maps it to its error codes.
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

#[derive(Debug, thiserror::Error)]
pub enum GuardError {
    #[error(transparent)]
    Refused(#[from] Refused),
    #[error("console audit write failed")]
    Database(#[from] DbErr),
}

type Decide<T> = Box<
    dyn FnOnce(Option<Role>, Option<&T>, Option<&T>) -> Result<NewConsoleAudit, Refused> + Send,
>;

struct Inner<T> {
    log: ConsoleAuditLog,
    actor: Option<i64>,
    decide: Decide<T>,
}

/// `before` is `None` for a creation, `after` `None` for a deletion.
pub struct TxGuard<T> {
    inner: Option<Inner<T>>,
}

impl<T> TxGuard<T> {
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

    /// The services' own entry points (login, provisioning, seeding), which are not console actions.
    pub(crate) fn none() -> Self {
        Self { inner: None }
    }

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

/// Share-locked on PostgreSQL; SQLite's write transaction already excludes other writers.
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
