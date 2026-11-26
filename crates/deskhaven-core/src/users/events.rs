//! Hooks and events run only after commit, from a task the caller cannot cancel. Hooks run
//! synchronously before the writing call returns; events are a broadcast channel. A lagging
//! receiver has lost events and must treat every user as affected. Events are not persisted, so
//! anything that must survive a crash between commit and delivery is also enforced at read time.
//! Role changes emit no event: authorization must read the current role, never one captured at
//! login.

use std::sync::Arc;

pub(super) const EVENT_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserEvent {
    Disabled { user_id: i64 },
    Deleted { user_id: i64 },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UserChange {
    Event(UserEvent),
    RoleChanged { user_id: i64 },
    StatusChanged { user_id: i64 },
    AccountChanged { user_id: i64 },
}

impl UserChange {
    pub fn user_id(&self) -> i64 {
        match *self {
            Self::Event(event) => event.user_id(),
            Self::RoleChanged { user_id }
            | Self::StatusChanged { user_id }
            | Self::AccountChanged { user_id } => user_id,
        }
    }
}

pub type CommitHook = Arc<dyn Fn(&UserChange) + Send + Sync>;
