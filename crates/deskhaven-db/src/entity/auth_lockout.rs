//! `auth_lockouts`: per-key login failure counter and exponential-backoff lockout.
//! No foreign key to `users`: unknown usernames are counted the same way (anti-enumeration).

use sea_orm::entity::prelude::*;

use crate::time::TimestampMs;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "auth_lockouts")]
pub struct Model {
    /// `user:<username_norm>`; built only by `deskhaven_core::auth::limiter::LockoutKey`.
    #[sea_orm(primary_key, auto_increment = false, column_type = "Text")]
    pub key: String,
    /// Failures counted in the current window (reset when a lock starts).
    pub failures: i32,
    pub window_started_at: TimestampMs,
    /// End of the latest lock, kept after it passed to drive the backoff level.
    pub locked_until: Option<TimestampMs>,
    /// Backoff level of the latest lock: it lasted `duration × 2^lock_level` (capped).
    pub lock_level: i32,
    pub updated_at: TimestampMs,
    /// Source rows only: last successful login of this username from this source. Within
    /// the known-source retention it lets the source bypass the global row.
    pub last_success_at: Option<TimestampMs>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
