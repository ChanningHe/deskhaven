//! Shared setup: a device service wired like `AppState` (strategy cache watching users and
//! groups, one manual clock), and registered devices.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test module uses a subset.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use base64_022::Engine as _;
use base64_022::engine::general_purpose::STANDARD;
use deskhaven_config::settings::{Change, PinSet};
use deskhaven_core::addressbooks::{AddressBookService, Book, NewSharedBook};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::devices::{
    DeviceRepository, DeviceService, Presence, Presets, Registration, Sysinfo, SysinfoOutcome,
    SysinfoReport,
};
use deskhaven_core::groups::GroupService;
use deskhaven_core::settings::{SettingsService, UpdateRequest};
use deskhaven_core::strategies::{NewStrategy, Options, Strategy, StrategyService};
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, User, UserService,
};
use deskhaven_db::entity::device;
use deskhaven_db::testing::TestDb;
use deskhaven_db::time;
use sea_orm::DatabaseConnection;

pub const T0_MS: i64 = 1_700_000_000_123;
pub const ID: &str = "123456789";

const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

pub struct Ctx {
    pub conn: DatabaseConnection,
    pub clock: Arc<ManualClock>,
    pub settings: SettingsService,
    pub users: UserService,
    pub groups: GroupService,
    pub strategies: StrategyService,
    pub books: AddressBookService,
    pub devices: DeviceService,
}

pub async fn ctx(db: &TestDb) -> Ctx {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto.clone(), PinSet::empty())
        .await
        .unwrap();
    let clock = Arc::new(ManualClock::at(T0_MS));
    let hasher = PasswordHasher::new(FAST).unwrap();
    let users = UserService::with_clock(db.conn.clone(), settings.clone(), hasher, clock.clone());
    let groups = GroupService::new(db.conn.clone());
    let strategies = StrategyService::new(db.conn.clone(), clock.clone());
    strategies.watch_users(&users);
    strategies.watch_groups(&groups);
    let books = AddressBookService::new(db.conn.clone(), settings.clone(), crypto.clone());
    let devices = DeviceService::new(
        db.conn.clone(),
        settings.clone(),
        crypto,
        strategies.clone(),
        Presence::new(clock.clone()),
    );
    Ctx {
        conn: db.conn.clone(),
        clock,
        settings,
        users,
        groups,
        strategies,
        books,
        devices,
    }
}

/// Base64 of `uuid`, as the client sends it (`crate::encode64(get_uuid())`).
pub fn b64(uuid: &[u8]) -> String {
    STANDARD.encode(uuid)
}

/// The uuid every [`Ctx::device`] registers with: derived from the id.
pub fn uuid_of(rustdesk_id: &str) -> Vec<u8> {
    format!("uuid-of-{rustdesk_id}").into_bytes()
}

pub fn sysinfo(hostname: &str) -> Sysinfo {
    Sysinfo {
        hostname: Some(hostname.to_owned()),
        username: Some("alice".to_owned()),
        os: Some("linux / Ubuntu 24.04".to_owned()),
        cpu: Some("Intel, 3.2GHz, 8/4 cores".to_owned()),
        memory: Some("15.5GB".to_owned()),
        version: Some("1.4.1".to_owned()),
    }
}

pub fn report(rustdesk_id: &str, info: Sysinfo, presets: Presets) -> SysinfoReport {
    SysinfoReport {
        rustdesk_id: rustdesk_id.to_owned(),
        uuid: b64(&uuid_of(rustdesk_id)),
        sysinfo: info,
        presets,
    }
}

impl Ctx {
    /// Registers a device (as the signal server would) and returns its `devices.id`.
    pub async fn device(&self, rustdesk_id: &str) -> i64 {
        let repo = DeviceRepository::new(self.conn.clone());
        let registration = Registration {
            rustdesk_id: rustdesk_id.to_owned(),
            uuid: uuid_of(rustdesk_id),
            pk: vec![9; 32],
            ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            at: time::from_millis(T0_MS).unwrap(),
        };
        repo.upsert_registration(&registration).await.unwrap();
        self.row(rustdesk_id).await.id
    }

    pub async fn row(&self, rustdesk_id: &str) -> device::Model {
        DeviceRepository::new(self.conn.clone())
            .find_by_rustdesk_id(rustdesk_id)
            .await
            .unwrap()
            .unwrap()
    }

    pub async fn report(&self, report: SysinfoReport) -> SysinfoOutcome {
        self.devices.report_sysinfo(report).await.unwrap()
    }

    pub async fn user(&self, name: &str) -> User {
        let credential = Credential::Local {
            password: Password::new("tundra-Lamp-42"),
        };
        self.users
            .create(NewUser::new(name, credential))
            .await
            .unwrap()
    }

    pub async fn strategy(&self, name: &str) -> Strategy {
        let options: Options = [("enable-file-transfer".to_owned(), "N".to_owned())]
            .into_iter()
            .collect();
        let new = NewStrategy::new(name, options);
        self.strategies.create(new).await.unwrap()
    }

    pub async fn shared_book(&self, name: &str) -> Book {
        let new = NewSharedBook {
            name: name.to_owned(),
            note: String::new(),
            owner_user_id: None,
        };
        self.books.create_shared(new).await.unwrap()
    }

    pub async fn set(&self, key: &str, value: serde_json::Value) {
        let version = self.settings.snapshot().view(key).unwrap().version;
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
        self.settings.update(request).await.unwrap();
    }
}
