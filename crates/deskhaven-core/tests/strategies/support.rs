//! Shared setup: services over one clock, and rows the strategy service refers to.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic)]
// Each test module uses a subset.
#![allow(dead_code)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::devices::{DeviceRepository, Registration};
use deskhaven_core::groups::{Group, GroupService, NewGroup};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::strategies::{
    DeviceInputs, NewStrategy, Options, Strategy, StrategyService, Target,
};
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, User, UserService,
};
use deskhaven_db::entity::device_group;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::testing::TestDb;
use deskhaven_db::time;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};

/// A fixed instant with a non-zero millisecond part.
pub const T0_MS: i64 = 1_700_000_000_123;

const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

pub struct Ctx {
    pub conn: DatabaseConnection,
    pub clock: Arc<ManualClock>,
    pub svc: StrategyService,
    pub users: UserService,
    pub groups: GroupService,
}

/// Strategy, user and group services sharing one manual clock, with the strategy cache
/// watching users and groups.
pub async fn ctx(db: &TestDb) -> Ctx {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
        .await
        .unwrap();
    let clock = Arc::new(ManualClock::at(T0_MS));
    let hasher = PasswordHasher::new(FAST).unwrap();
    let users = UserService::with_clock(db.conn.clone(), settings, hasher, clock.clone());
    let groups = GroupService::new(db.conn.clone());
    let svc = StrategyService::new(db.conn.clone(), clock.clone());
    svc.watch_users(&users);
    svc.watch_groups(&groups);
    Ctx {
        conn: db.conn.clone(),
        clock,
        svc,
        users,
        groups,
    }
}

pub fn opts(pairs: &[(&str, &str)]) -> Options {
    let own = |(k, v): &(&str, &str)| ((*k).to_owned(), (*v).to_owned());
    pairs.iter().map(own).collect()
}

impl Ctx {
    pub async fn strategy(&self, name: &str, pairs: &[(&str, &str)]) -> Strategy {
        let new = NewStrategy::new(name, opts(pairs));
        self.svc.create(new).await.unwrap()
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

    pub async fn group(&self, name: &str) -> Group {
        self.groups.create(NewGroup::new(name)).await.unwrap()
    }

    pub async fn device_group(&self, name: &str) -> i64 {
        let now = time::now();
        let row = device_group::ActiveModel {
            name: Set(name.to_owned()),
            note: Set(String::new()),
            managed_by: Set(ManagedBy::Ui),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        row.insert(&self.conn).await.unwrap().id
    }

    /// Registers a device and returns its `devices.id`.
    pub async fn device(&self, rustdesk_id: &str) -> i64 {
        let repo = DeviceRepository::new(self.conn.clone());
        let registration = Registration {
            rustdesk_id: rustdesk_id.to_owned(),
            uuid: rustdesk_id.as_bytes().to_vec(),
            pk: b"pk".to_vec(),
            ip: IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            at: time::from_millis(T0_MS).unwrap(),
        };
        repo.upsert_registration(&registration).await.unwrap();
        repo.find_by_rustdesk_id(rustdesk_id)
            .await
            .unwrap()
            .unwrap()
            .id
    }

    pub async fn assign(&self, target: Target, strategy: &Strategy) {
        self.svc.assign(target, strategy.id).await.unwrap();
    }

    /// Effective options of a device, as a plain map.
    pub async fn options(&self, device: &DeviceInputs) -> Options {
        self.svc.effective(device).await.unwrap().options
    }
}

pub fn device_inputs(
    device_id: i64,
    user: Option<&User>,
    device_group: Option<i64>,
) -> DeviceInputs {
    DeviceInputs {
        device_id,
        user_id: user.map(|u| u.id),
        device_group_id: device_group,
        changed_at: None,
    }
}
