//! Shared setup: the application state on one manual clock, and rows of the tables the jobs
//! clean, inserted with chosen timestamps.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test module uses a subset.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use deskhaven_config::settings::{Change, PinSet};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::devices::{DeviceRepository, Registration};
use deskhaven_core::settings::{SettingsService, UpdateRequest};
use deskhaven_core::state::{AppState, StateConfig};
use deskhaven_core::users::{HashParams, PasswordHasher};
use deskhaven_db::entity::{
    alarm, audit_nonce, connection_session, console_audit, device_strategy_key, file_transfer,
    login_event,
};
use deskhaven_db::testing::TestDb;
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, PaginatorTrait};
use serde_json::Value as Json;

/// A fixed instant with a non-zero millisecond part.
pub const T0_MS: i64 = 1_700_000_000_123;
pub const HOUR: Duration = Duration::from_secs(3600);
pub const DAY: Duration = Duration::from_secs(24 * 3600);

const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

pub struct Ctx {
    pub conn: DatabaseConnection,
    pub clock: Arc<ManualClock>,
    pub state: Arc<AppState>,
}

pub async fn ctx(db: &TestDb) -> Ctx {
    let conn = db.conn.clone();
    let clock = Arc::new(ManualClock::at(T0_MS));
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(conn.clone(), crypto.clone(), PinSet::empty())
        .await
        .unwrap();
    let state = AppState::new(
        conn.clone(),
        settings,
        crypto,
        "pk".to_owned(),
        PasswordHasher::new(FAST).unwrap(),
        clock.clone(),
        &StateConfig::default(),
    );
    Ctx {
        conn,
        clock,
        state: Arc::new(state),
    }
}

/// `T0_MS` minus `age`.
pub fn ago(age: Duration) -> TimestampMs {
    at(T0_MS - i64::try_from(age.as_millis()).unwrap())
}

pub fn at(ms: i64) -> TimestampMs {
    time::from_millis(ms).unwrap()
}

impl Ctx {
    /// Sets `key` to `value`, whatever its current version.
    pub async fn set(&self, key: &str, value: Json) {
        let settings = &self.state.settings;
        let version = settings.snapshot().view(key).unwrap().version;
        let change = Change {
            key: key.to_owned(),
            value,
            version,
        };
        let request = UpdateRequest {
            changes: vec![change],
            actor: None,
            reason: None,
        };
        settings.update(request).await.unwrap();
    }

    pub async fn register(&self, rustdesk_id: &str) {
        let registration = Registration {
            rustdesk_id: rustdesk_id.to_owned(),
            uuid: format!("uuid-of-{rustdesk_id}").into_bytes(),
            pk: vec![9; 32],
            ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            at: at(T0_MS),
        };
        DeviceRepository::new(self.conn.clone())
            .upsert_registration(&registration)
            .await
            .unwrap();
    }

    /// A connection row with `session_id`, created at `created_at`.
    pub async fn connection(&self, session_id: &str, created_at: TimestampMs) -> i64 {
        let row = connection_session::ActiveModel {
            guid: Set(uuid::Uuid::now_v7().to_string()),
            device_rustdesk_id: Set("123456789".to_owned()),
            conn_id: Set(1),
            session_id: Set(session_id.to_owned()),
            created_at: Set(created_at),
            ..Default::default()
        };
        row.insert(&self.conn).await.unwrap().id
    }

    pub async fn file(&self, created_at: TimestampMs) {
        file_transfer::ActiveModel {
            device_rustdesk_id: Set("123456789".to_owned()),
            peer_rustdesk_id: Set("987654321".to_owned()),
            conn_id: Set(1),
            direction: Set(file_transfer::Direction::Send),
            path: Set("/tmp/a".to_owned()),
            is_file: Set(true),
            file_count: Set(1),
            files: Set("[]".to_owned()),
            created_at: Set(created_at),
            ..Default::default()
        }
        .insert(&self.conn)
        .await
        .unwrap();
    }

    pub async fn alarm(&self, created_at: TimestampMs) {
        alarm::ActiveModel {
            device_rustdesk_id: Set("123456789".to_owned()),
            typ: Set(1),
            info: Set("{}".to_owned()),
            conn_id: Set(1),
            created_at: Set(created_at),
            ..Default::default()
        }
        .insert(&self.conn)
        .await
        .unwrap();
    }

    pub async fn login(&self, created_at: TimestampMs) {
        login_event::ActiveModel {
            username_attempted: Set("alice".to_owned()),
            method: Set(login_event::Method::Password),
            result: Set("ok".to_owned()),
            client: Set(login_event::Client::Console),
            created_at: Set(created_at),
            ..Default::default()
        }
        .insert(&self.conn)
        .await
        .unwrap();
    }

    pub async fn console(&self, action: &str, created_at: TimestampMs) {
        console_audit::ActiveModel {
            actor_kind: Set(console_audit::ActorKind::Cli),
            action: Set(action.to_owned()),
            target_type: Set(String::new()),
            target_id: Set(String::new()),
            created_at: Set(created_at),
            ..Default::default()
        }
        .insert(&self.conn)
        .await
        .unwrap();
    }

    pub async fn nonce(&self, nonce: &str, received_at: TimestampMs) {
        audit_nonce::ActiveModel {
            nonce: Set(nonce.to_owned()),
            received_at: Set(received_at),
        }
        .insert(&self.conn)
        .await
        .unwrap();
    }

    pub async fn strategy_keys(&self, rustdesk_id: &str, updated_at: TimestampMs) {
        device_strategy_key::ActiveModel {
            rustdesk_id: Set(rustdesk_id.to_owned()),
            option_keys: Set(r#"["a"]"#.to_owned()),
            updated_at: Set(updated_at),
        }
        .insert(&self.conn)
        .await
        .unwrap();
    }

    pub async fn count<E>(&self, entity: E) -> u64
    where
        E: EntityTrait,
        E::Model: Sync,
    {
        let _ = entity;
        E::find().count(&self.conn).await.unwrap()
    }

    /// Console audit rows with `action`, oldest first.
    pub async fn audit_rows(&self, action: &str) -> Vec<console_audit::Model> {
        use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
        console_audit::Entity::find()
            .filter(console_audit::Column::Action.eq(action))
            .order_by_asc(console_audit::Column::Id)
            .all(&self.conn)
            .await
            .unwrap()
    }
}
