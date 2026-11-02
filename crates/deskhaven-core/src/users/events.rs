//! Account changes other components must react to (06 §3.2: password change, disable and
//! delete revoke every session of the user; the hbbs ACL cache follows).
//!
//! Mechanism: a `tokio::sync::broadcast` channel owned by [`UserService`](super::UserService).
//! Core does not know its consumers (session service, session cache, ACL cache, audit), any
//! number can subscribe, and sending never blocks the writer. Events are sent only after the
//! change has committed, from a task the caller cannot cancel half-way.
//!
//! Contract for subscribers: a lagging receiver gets `RecvError::Lagged` and has lost events;
//! it must then treat **every** user as affected (e.g. flush its whole cache). Events are not
//! persisted: anything that must survive a crash between commit and delivery (sessions in the
//! database) must also be enforced at read time (e.g. reject sessions of disabled users).

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
