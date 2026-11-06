//! One simulated device: its identity and the task that registers it with the signal
//! server and runs its client API timer ([`crate::plan`]).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use ed25519_dalek::SigningKey;
use hyper::Method;
use serde_json::Value;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::LoadgenError;
use crate::http::{self, Reply};
use crate::plan::{AuditCall, DeviceState, HeartbeatReply, Rates, SysinfoOutcome, Timing};
use crate::rendezvous::{Keys, Rendezvous};
use crate::requests::{self, Sender};
use crate::stats::{Failure, Kind, Recorder};

/// RustDesk ids of the simulated devices start here (nine digits, like real ones).
const FIRST_ID: u64 = 100_000_000;

/// Who a simulated device is; derived from its index, so runs are reproducible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub index: usize,
    pub keys: Keys,
    /// The uuid as the client sends it to the API (`encode64(get_uuid())`).
    pub uuid_b64: String,
    /// Its own loopback source address, for UDP and TCP.
    pub ip: Ipv4Addr,
}

impl Identity {
    pub fn new(index: usize, first_ip: Ipv4Addr) -> Result<Self, LoadgenError> {
        let ip = u32::try_from(index)
            .ok()
            .and_then(|offset| u32::from(first_ip).checked_add(offset))
            .map(Ipv4Addr::from)
            .filter(Ipv4Addr::is_loopback)
            .ok_or_else(|| {
                LoadgenError::Config(format!("device {index} from {first_ip} leaves 127.0.0.0/8"))
            })?;
        let number = u64::try_from(index).unwrap_or(u64::MAX);
        let uuid = format!("loadgen-{index:010}").into_bytes();
        let mut seed = [0x5a_u8; 32];
        seed[..8].copy_from_slice(&number.to_le_bytes());
        let pk = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
        Ok(Self {
            index,
            uuid_b64: STANDARD.encode(&uuid),
            keys: Keys {
                id: FIRST_ID.saturating_add(number).to_string(),
                uuid,
                pk: pk.to_vec(),
            },
            ip,
        })
    }

    fn sender(&self) -> Sender<'_> {
        Sender {
            id: &self.keys.id,
            uuid_b64: &self.uuid_b64,
        }
    }
}

/// What every device task shares.
pub struct Shared {
    pub api: SocketAddr,
    pub signal: SocketAddr,
    pub timing: Timing,
    pub rates: Rates,
    pub recorder: Arc<Recorder>,
    pub stop: CancellationToken,
    pub http_timeout: Duration,
    /// How long a registration waits for its answer (`MIN_REG_TIMEOUT`, 3 s).
    pub udp_timeout: Duration,
}

/// The registration channel (`RendezvousMediator`): its own task and UDP socket.
struct Signal {
    identity: Identity,
    shared: Arc<Shared>,
    rendezvous: Rendezvous,
    key_confirmed: bool,
}

/// The client API timer (`start_hbbs_sync`): its own task, independent of registration.
struct Api {
    identity: Identity,
    shared: Arc<Shared>,
}

/// Waits for `start_at`; `false` when the run stopped first.
async fn started(shared: &Shared, start_at: Instant) -> bool {
    tokio::select! {
        () = shared.stop.cancelled() => false,
        () = sleep_until(start_at) => true,
    }
}

/// Registers device `identity` from `start_at` until `shared.stop`: `RegisterPk` first (a
/// fresh install), then `RegisterPeer` every `register` interval.
pub async fn run_signal(identity: Identity, shared: Arc<Shared>, start_at: Instant) {
    if !started(&shared, start_at).await {
        return;
    }
    let rendezvous = match Rendezvous::bind(IpAddr::V4(identity.ip), shared.signal).await {
        Ok(rendezvous) => rendezvous,
        Err(err) => {
            // The device never registers: counted whatever the phase, so the run fails.
            tracing::warn!(ip = %identity.ip, error = %err, "cannot bind the device socket");
            shared.recorder.record_start_failure();
            return;
        }
    };
    let period = shared.timing.register;
    let stop = shared.stop.clone();
    let mut signal = Signal {
        identity,
        shared,
        rendezvous,
        key_confirmed: false,
    }
    .register()
    .await;
    let mut register = interval_at(start_at + period, period);
    register.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => break,
            _ = register.tick() => signal = signal.register().await,
        }
    }
}

/// Runs the client API timer of device `identity` from `start_at` until `shared.stop`.
pub async fn run_api(identity: Identity, shared: Arc<Shared>, start_at: Instant, seed: u64) {
    if !started(&shared, start_at).await {
        return;
    }
    let timing = shared.timing;
    let stop = shared.stop.clone();
    let mut state = DeviceState::new(start_at, timing, shared.rates, seed);
    let api = Api { identity, shared };
    let mut tick = interval_at(start_at + timing.tick, timing.tick);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            () = stop.cancelled() => break,
            at = tick.tick() => state = api.on_tick(state, at).await,
        }
    }
}

impl Signal {
    /// `RegisterPeer`, or `RegisterPk` while the key is unconfirmed or when asked for it.
    async fn register(self) -> Self {
        if !self.key_confirmed {
            return self.register_pk().await;
        }
        let started = std::time::Instant::now();
        let wait = self.shared.udp_timeout;
        let result = self
            .rendezvous
            .register_peer(&self.identity.keys.id, wait)
            .await;
        let request_pk = matches!(result, Ok(true));
        record(&self.shared, Kind::RegisterPeer, started, result);
        match request_pk {
            true => self.register_pk().await,
            false => self,
        }
    }

    async fn register_pk(self) -> Self {
        let started = std::time::Instant::now();
        let wait = self.shared.udp_timeout;
        let result = self.rendezvous.register_pk(&self.identity.keys, wait).await;
        let key_confirmed = result.is_ok();
        record(&self.shared, Kind::RegisterPk, started, result);
        Self {
            key_confirmed,
            ..self
        }
    }
}

impl Api {
    async fn on_tick(&self, state: DeviceState, at: Instant) -> DeviceState {
        let (state, plan) = state.tick(at);
        let state = match plan.sysinfo {
            true => state.sysinfo_done(at, self.sysinfo().await),
            false => state,
        };
        let state = match plan.heartbeat {
            Some(conns) => {
                let reply = self.heartbeat(&conns, state.modified_at()).await;
                state.heartbeat_done(reply)
            }
            None => state,
        };
        if let Some(call) = plan.audit {
            self.audit(&call).await;
        }
        state
    }

    async fn sysinfo(&self) -> SysinfoOutcome {
        let body = requests::sysinfo(self.identity.sender(), self.identity.index);
        let (started, reply) = self.post("/api/sysinfo", &body).await;
        let (outcome, result) = match reply {
            Ok(reply) => requests::sysinfo_reply(reply.status, &reply.body),
            Err(failure) => (SysinfoOutcome::Other, Err(failure)),
        };
        record(&self.shared, Kind::Sysinfo, started, result);
        outcome
    }

    /// The reply's keys; a failed heartbeat changes nothing on the client.
    async fn heartbeat(&self, conns: &[i32], modified_at: i64) -> HeartbeatReply {
        let body = requests::heartbeat(self.identity.sender(), conns, modified_at);
        let (started, reply) = self.post("/api/heartbeat", &body).await;
        let result = reply.and_then(|reply| requests::heartbeat_reply(reply.status, &reply.body));
        let parsed = result.clone().unwrap_or_default();
        record(&self.shared, Kind::Heartbeat, started, result);
        parsed
    }

    async fn audit(&self, call: &AuditCall) {
        let nonce = uuid::Uuid::new_v4().to_string();
        let body = requests::audit(self.identity.sender(), call, &nonce);
        let (started, reply) = self.post("/api/audit/conn", &body).await;
        let result = reply.and_then(|reply| requests::audit_reply(reply.status, &reply.body));
        record(&self.shared, Kind::Audit, started, result);
    }

    async fn post(&self, path: &str, body: &Value) -> (std::time::Instant, Result<Reply, Failure>) {
        let started = std::time::Instant::now();
        let local = IpAddr::V4(self.identity.ip);
        let (api, limit) = (self.shared.api, self.shared.http_timeout);
        let json = Some(body.to_string());
        let reply = http::send(local, api, Method::POST, path, json, limit).await;
        (started, reply)
    }
}

/// Files a finished call, its latency measured from `started`.
fn record<T>(shared: &Shared, kind: Kind, started: std::time::Instant, result: Result<T, Failure>) {
    let latency = started.elapsed();
    shared.recorder.record(kind, latency, result.map(|_| ()));
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_IP: Ipv4Addr = Ipv4Addr::new(127, 64, 0, 1);

    #[test]
    fn identities_are_distinct_reproducible_and_loopback() {
        let a = Identity::new(0, FIRST_IP).unwrap();
        let b = Identity::new(300, FIRST_IP).unwrap();

        assert_eq!(a, Identity::new(0, FIRST_IP).unwrap());
        assert_eq!(a.keys.id, "100000000");
        assert_eq!(b.keys.id, "100000300");
        assert_eq!(b.ip, Ipv4Addr::new(127, 64, 1, 45));
        assert_ne!(a.keys.uuid, b.keys.uuid);
        assert_ne!(a.keys.pk, b.keys.pk);
        assert_eq!(a.keys.pk.len(), 32);
        assert_eq!(STANDARD.decode(&a.uuid_b64).unwrap(), a.keys.uuid);
    }

    #[test]
    fn addresses_must_stay_in_the_loopback_net() {
        let last = Ipv4Addr::new(127, 255, 255, 254);

        assert!(Identity::new(1, last).is_ok());
        assert!(matches!(
            Identity::new(2, last),
            Err(LoadgenError::Config(_))
        ));
    }
}
