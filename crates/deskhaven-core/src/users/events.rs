//! Account changes other components must react to (06 §3.2: password change, disable and
//! delete revoke every session of the user; the hbbs ACL cache follows).
//!
//! Two mechanisms, both run only after the change has committed, from a task the caller
//! cannot cancel half-way:
//!
//! 1. **Commit hooks** ([`UserService::register_commit_hook`](super::UserService::register_commit_hook)):
//!    called synchronously with a [`UserChange`] before the writing call returns, for state
//!    that must be consistent the moment it does (cache invalidation). Hooks must be fast and
//!    must not block or panic. Role and status changes reach hooks too, so cached roles and
//!    statuses never outlive the call that changed them.
//! 2. **Events** ([`UserService::subscribe`](super::UserService::subscribe)): a
//!    `tokio::sync::broadcast` channel of [`UserEvent`], sent right after the hooks. Core does
//!    not know its consumers (session service, ACL, audit), any number can subscribe, and
//!    sending never blocks the writer.
//!
//! Contract for subscribers: a lagging receiver gets `RecvError::Lagged` and has lost events;
//! it must then treat **every** user as affected (e.g. flush its whole cache). Events are not
//! persisted: anything that must survive a crash between commit and delivery (sessions in the
//! database) must also be enforced at read time (e.g. reject sessions of disabled users).
//!
//! Role changes emit **no** event: sessions stay valid across a promotion or demotion, so
//! authorization (RBAC) must read the user's current role on every request (or from a cache
//! no older than its TTL, invalidated through a commit hook), never from a value captured at
//! login.

use std::sync::Arc;

/// Capacity of the event channel before slow receivers start lagging.
pub(super) const EVENT_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserEvent {
    /// The account went to `disabled` (sent only on the transition).
    Disabled { user_id: i64 },
    /// The account and, by cascade, its memberships, sessions and tokens are gone.
    Deleted { user_id: i64 },
    /// The local password was replaced.
    PasswordChanged { user_id: i64 },
}

impl UserEvent {
    pub fn user_id(&self) -> i64 {
        match *self {
            Self::Disabled { user_id }
            | Self::Deleted { user_id }
            | Self::PasswordChanged { user_id } => user_id,
        }
    }
}

/// A committed change that cached views of the user must drop, as passed to commit hooks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserChange {
    /// Also broadcast as this event, right after the hooks.
    Event(UserEvent),
    /// The role changed (no event).
    RoleChanged { user_id: i64 },
    /// The status changed to anything but `disabled` (that one is `Event(Disabled)`).
    StatusChanged { user_id: i64 },
}

impl UserChange {
    pub fn user_id(&self) -> i64 {
        match *self {
            Self::Event(event) => event.user_id(),
            Self::RoleChanged { user_id } | Self::StatusChanged { user_id } => user_id,
        }
    }
}

/// Called synchronously after each committed [`UserChange`].
pub type CommitHook = Arc<dyn Fn(&UserChange) + Send + Sync>;
