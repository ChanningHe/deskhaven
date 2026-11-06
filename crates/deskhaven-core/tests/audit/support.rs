//! Shared setup: an audit service over a device registry on one manual clock, registered
//! devices, and builders of the messages the client sends.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test module uses a subset.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use deskhaven_config::settings::PinSet;
use deskhaven_core::audit::{
    AuditService, Authorization, ConnEvent, ConnReport, ControllerNote, Envelope, Ingest, Record,
    SessionId,
};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::devices::{DeviceRepository, DeviceService, Registration};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::strategies::StrategyService;
use deskhaven_db::entity::{alarm, audit_nonce, connection_session, file_transfer};
use deskhaven_db::testing::TestDb;
use deskhaven_db::time;
use sea_orm::{DatabaseConnection, EntityTrait, PaginatorTrait, QueryOrder};

pub const T0_MS: i64 = 1_700_000_000_123;
pub const ID: &str = "123456789";
/// The controller's session id, as the authorization and close messages carry it.
pub const SESSION: u64 = 8_123_456_789_012_345_678;
pub const PEER: &str = "987654321";

pub struct Ctx {
    pub conn: DatabaseConnection,
    pub clock: Arc<ManualClock>,
    pub audit: AuditService,
}

pub async fn ctx(db: &TestDb) -> Ctx {
    let conn = db.conn.clone();
    let clock = Arc::new(ManualClock::at(T0_MS));
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(conn.clone(), crypto.clone(), PinSet::empty())
        .await
        .unwrap();
    let strategies = StrategyService::new(conn.clone(), crypto.clone(), clock.clone());
    let devices = DeviceService::new(conn.clone(), settings, crypto, strategies, clock.clone());
    let audit = AuditService::new(conn.clone(), devices, clock.clone());
    let c = Ctx { conn, clock, audit };
    c.register(ID).await;
    c
}

/// Base64 of the uuid [`Ctx::register`] registers `rustdesk_id` with, as the client sends it.
pub fn uuid_b64(rustdesk_id: &str) -> String {
    STANDARD.encode(format!("uuid-of-{rustdesk_id}"))
}

pub fn envelope(nonce: &str) -> Envelope {
    Envelope {
        rustdesk_id: ID.to_owned(),
        uuid: uuid_b64(ID),
        nonce: Some(nonce.to_owned()),
    }
}

/// A client at 198.51.100.`n`.
pub fn client(n: u8) -> ClientAddr {
    ClientAddr {
        addr: IpAddr::V4(Ipv4Addr::new(198, 51, 100, n)),
        is_proxy_fallback: false,
    }
}

pub fn note(rustdesk_id: &str, session_id: u64, text: &str) -> ControllerNote {
    ControllerNote {
        rustdesk_id: rustdesk_id.to_owned(),
        session_id: session(session_id),
        note: text.to_owned(),
    }
}

pub fn session(id: u64) -> SessionId {
    SessionId::from_json(&serde_json::json!(id)).unwrap()
}

/// The first message: sent when the connection opens, before the login request, so its
/// session id is still 0 (`src/server/connection.rs:1527-1534`).
pub fn new_conn(conn_id: i64) -> Record {
    Record::Conn(ConnReport {
        conn_id,
        session_id: session(0),
        event: ConnEvent::New {
            ip: Some(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 4))),
        },
    })
}

pub fn authorized(conn_id: i64, session_id: u64) -> Record {
    Record::Conn(ConnReport {
        conn_id,
        session_id: session(session_id),
        event: ConnEvent::Authorized(Authorization {
            peer_id: PEER.to_owned(),
            peer_name: "alice@laptop".to_owned(),
            conn_type: 0,
            primary_auth: Some(2),
            two_factor: Some(1),
        }),
    })
}

pub fn closed(conn_id: i64, session_id: u64) -> Record {
    Record::Conn(ConnReport {
        conn_id,
        session_id: session(session_id),
        event: ConnEvent::Close,
    })
}

impl Ctx {
    pub async fn register(&self, rustdesk_id: &str) {
        let registration = Registration {
            rustdesk_id: rustdesk_id.to_owned(),
            uuid: format!("uuid-of-{rustdesk_id}").into_bytes(),
            pk: vec![9; 32],
            ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            at: time::from_millis(T0_MS).unwrap(),
        };
        DeviceRepository::new(self.conn.clone())
            .upsert_registration(&registration)
            .await
            .unwrap();
    }

    /// Ingests `record` from device [`ID`] under `nonce`.
    pub async fn send(&self, nonce: &str, record: Record) -> Ingest {
        self.audit.ingest(envelope(nonce), record).await.unwrap()
    }

    /// Ingests `record` from the registered device `rustdesk_id` under `nonce`.
    pub async fn send_from(&self, rustdesk_id: &str, nonce: &str, record: Record) -> Ingest {
        let envelope = Envelope {
            rustdesk_id: rustdesk_id.to_owned(),
            uuid: uuid_b64(rustdesk_id),
            nonce: Some(nonce.to_owned()),
        };
        self.audit.ingest(envelope, record).await.unwrap()
    }

    pub async fn sessions(&self) -> Vec<connection_session::Model> {
        connection_session::Entity::find()
            .order_by_asc(connection_session::Column::Id)
            .all(&self.conn)
            .await
            .unwrap()
    }

    pub async fn files(&self) -> Vec<file_transfer::Model> {
        file_transfer::Entity::find().all(&self.conn).await.unwrap()
    }

    pub async fn alarms(&self) -> Vec<alarm::Model> {
        alarm::Entity::find().all(&self.conn).await.unwrap()
    }

    pub async fn nonces(&self) -> u64 {
        audit_nonce::Entity::find().count(&self.conn).await.unwrap()
    }
}
