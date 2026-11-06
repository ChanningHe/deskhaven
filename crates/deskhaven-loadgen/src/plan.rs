//! What one simulated device sends on each tick of its client API timer: the decisions of
//! `start_hbbs_sync_async` (`src/hbbs_http/sync.rs:87-280`, client `23e5bb4`) plus the audit
//! records of an occasional connection (`src/server/connection.rs`, 03 §2.3).
//!
//! Pure state: the device task performs the [`Plan`] and feeds the outcomes back.

use std::time::Duration;

use tokio::time::Instant;

/// The client's timers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// `TIME_CONN`: the sync loop's tick.
    pub tick: Duration,
    /// `TIME_HEARTBEAT`: heartbeat period without live connections.
    pub heartbeat: Duration,
    /// `REG_INTERVAL` (`hbb_common::config`): RegisterPeer period.
    pub register: Duration,
    /// `UPLOAD_SYSINFO_TIMEOUT`: wait after a sysinfo answer other than the two known ones.
    pub sysinfo_retry: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(3),
            heartbeat: Duration::from_secs(15),
            register: Duration::from_secs(12),
            sysinfo_retry: Duration::from_secs(120),
        }
    }
}

/// Mean per-device arrival rates of the occasional calls (Poisson arrivals).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    /// Incoming connections per device per hour; each sends three audit records.
    pub connections_per_hour: f64,
    /// Sysinfo re-uploads per device per hour (a client re-uploads when its user changes).
    pub sysinfo_per_hour: f64,
}

/// Which audit record of a connection (03 §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditStep {
    New,
    Authorized,
    Close,
}

/// One `POST /api/audit/conn`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditCall {
    pub step: AuditStep,
    pub conn_id: i32,
    /// 0 for `new` (sent before the login request), the controller's session id after.
    pub session_id: u64,
}

/// What to send on this tick, in this order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub sysinfo: bool,
    /// The heartbeat's `conns`, when a heartbeat is due.
    pub heartbeat: Option<Vec<i32>>,
    pub audit: Option<AuditCall>,
}

/// How the client reads a sysinfo answer (`sync.rs:211-228`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SysinfoOutcome {
    /// `SYSINFO_UPDATED`: uploaded.
    Updated,
    /// `ID_NOT_FOUND`: try again on the next tick.
    IdNotFound,
    /// Anything else, a failure included: try again after [`Timing::sysinfo_retry`].
    Other,
}

/// The keys of a heartbeat reply the client acts on besides strategy and disconnect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeartbeatReply {
    pub sysinfo: bool,
    pub modified_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sysinfo {
    Needed,
    RetryAt(Instant),
    Uploaded,
}

#[derive(Clone, Copy, Debug)]
struct Connection {
    conn_id: i32,
    session_id: u64,
    next: AuditStep,
}

/// One device's client-side state.
#[derive(Clone, Debug)]
pub struct DeviceState {
    timing: Timing,
    rates: Rates,
    rng: fastrand::Rng,
    last_heartbeat: Option<Instant>,
    sysinfo: Sysinfo,
    refresh_at: Option<Instant>,
    connection: Option<Connection>,
    next_connection_at: Option<Instant>,
    next_conn_id: i32,
    modified_at: i64,
}

impl DeviceState {
    pub fn new(now: Instant, timing: Timing, rates: Rates, seed: u64) -> Self {
        let mut rng = fastrand::Rng::with_seed(seed);
        let next_connection_at = arrival(&mut rng, now, rates.connections_per_hour);
        let refresh_at = arrival(&mut rng, now, rates.sysinfo_per_hour);
        Self {
            timing,
            rates,
            rng,
            last_heartbeat: None,
            sysinfo: Sysinfo::Needed,
            refresh_at,
            connection: None,
            next_connection_at,
            next_conn_id: 1,
            modified_at: 0,
        }
    }

    /// The strategy version the client holds (`strategy_timestamp`), sent as `modified_at`.
    pub fn modified_at(&self) -> i64 {
        self.modified_at
    }

    /// Decides this tick's calls (`now` is the tick's scheduled instant).
    pub fn tick(self, now: Instant) -> (Self, Plan) {
        let (state, sysinfo) = self.sysinfo_due(now);
        let (state, audit) = state.audit_due(now);
        let conns: Vec<i32> = state.connection.iter().map(|c| c.conn_id).collect();
        let heartbeat_due = !conns.is_empty()
            || state
                .last_heartbeat
                .is_none_or(|last| now.saturating_duration_since(last) >= state.timing.heartbeat);
        let state = match heartbeat_due {
            true => Self {
                last_heartbeat: Some(now),
                ..state
            },
            false => state,
        };
        let plan = Plan {
            sysinfo,
            heartbeat: heartbeat_due.then_some(conns),
            audit,
        };
        (state, plan)
    }

    pub fn sysinfo_done(self, now: Instant, outcome: SysinfoOutcome) -> Self {
        let sysinfo = match outcome {
            SysinfoOutcome::Updated => Sysinfo::Uploaded,
            SysinfoOutcome::IdNotFound => Sysinfo::Needed,
            SysinfoOutcome::Other => Sysinfo::RetryAt(now + self.timing.sysinfo_retry),
        };
        Self { sysinfo, ..self }
    }

    pub fn heartbeat_done(self, reply: HeartbeatReply) -> Self {
        let sysinfo = match reply.sysinfo {
            true => Sysinfo::Needed,
            false => self.sysinfo,
        };
        Self {
            sysinfo,
            modified_at: reply.modified_at.unwrap_or(self.modified_at),
            ..self
        }
    }

    /// `need_upload` of `sync.rs:126-131`, plus the occasional re-upload.
    fn sysinfo_due(self, now: Instant) -> (Self, bool) {
        let refresh_due = self.refresh_at.is_some_and(|at| at <= now);
        let mut rng = self.rng.clone();
        let (sysinfo, refresh_at) = match (self.sysinfo, refresh_due) {
            (Sysinfo::Uploaded, true) => (
                Sysinfo::Needed,
                arrival(&mut rng, now, self.rates.sysinfo_per_hour),
            ),
            (current, _) => (current, self.refresh_at),
        };
        let due = match sysinfo {
            Sysinfo::Needed => true,
            Sysinfo::RetryAt(at) => at <= now,
            Sysinfo::Uploaded => false,
        };
        let state = Self {
            sysinfo,
            refresh_at,
            rng,
            ..self
        };
        (state, due)
    }

    /// The next record of the live connection, or a new connection when one is due.
    fn audit_due(self, now: Instant) -> (Self, Option<AuditCall>) {
        let mut rng = self.rng.clone();
        if let Some(conn) = self.connection {
            let call = AuditCall {
                step: conn.next,
                conn_id: conn.conn_id,
                session_id: conn.session_id,
            };
            let after = |next| Some(Connection { next, ..conn });
            let (connection, next_connection_at) = match conn.next {
                AuditStep::New => (after(AuditStep::Authorized), self.next_connection_at),
                AuditStep::Authorized => (after(AuditStep::Close), self.next_connection_at),
                AuditStep::Close => (
                    None,
                    arrival(&mut rng, now, self.rates.connections_per_hour),
                ),
            };
            let state = Self {
                connection,
                next_connection_at,
                rng,
                ..self
            };
            return (state, Some(call));
        }
        if !self.next_connection_at.is_some_and(|at| at <= now) {
            return (self, None);
        }
        self.open_connection(rng)
    }

    fn open_connection(self, mut rng: fastrand::Rng) -> (Self, Option<AuditCall>) {
        let conn_id = self.next_conn_id;
        // A controller session id: any non-zero 63-bit value.
        let session_id = rng.u64(1..=i64::MAX.unsigned_abs());
        let call = AuditCall {
            step: AuditStep::New,
            conn_id,
            session_id: 0,
        };
        let connection = Connection {
            conn_id,
            session_id,
            next: AuditStep::Authorized,
        };
        let state = Self {
            connection: Some(connection),
            next_connection_at: None,
            next_conn_id: conn_id.wrapping_add(1),
            rng,
            ..self
        };
        (state, Some(call))
    }
}

/// The next Poisson arrival after `now` at `per_hour`; `None` when the rate is not positive.
fn arrival(rng: &mut fastrand::Rng, now: Instant, per_hour: f64) -> Option<Instant> {
    if per_hour.is_nan() || per_hour <= 0.0 {
        return None;
    }
    // 1 - f64() is in (0, 1], so the logarithm is finite.
    let gap = -(1.0 - rng.f64()).ln() * 3600.0 / per_hour;
    Duration::try_from_secs_f64(gap)
        .ok()
        .and_then(|gap| now.checked_add(gap))
}

#[cfg(test)]
mod tests;
