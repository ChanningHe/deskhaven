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
use deskhaven_core::addressbooks::{
    Actor, AddressBookService, Book, NewSharedBook, Peer, Rule, Subject,
};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::devices::{
    DeviceRepository, DeviceService, Presets, Registration, SYSINFO_WRITE_INTERVAL, Sysinfo,
    SysinfoOutcome, SysinfoReport,
};
use deskhaven_core::groups::GroupService;
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::settings::{SettingsService, UpdateRequest};
use deskhaven_core::strategies::{NewStrategy, Options, Strategy, StrategyService};
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, User, UserService,
};
use deskhaven_db::entity::address_book_tag;
use deskhaven_db::entity::device;
use deskhaven_db::testing::TestDb;
use deskhaven_db::time;
use sea_orm::DatabaseConnection;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait, IntoActiveModel};

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
    ctx_on(db.conn.clone(), Arc::new(ManualClock::at(T0_MS))).await
}

/// Services over `conn` (e.g. a clone carrying a statement counter) on `clock`. Called twice
/// over one database, it is a restart.
pub async fn ctx_on(conn: DatabaseConnection, clock: Arc<ManualClock>) -> Ctx {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) = SettingsService::load(conn.clone(), crypto.clone(), PinSet::empty())
        .await
        .unwrap();
    let hasher = PasswordHasher::new(FAST).unwrap();
    let users = UserService::with_clock(conn.clone(), settings.clone(), hasher, clock.clone());
    let groups = GroupService::new(conn.clone());
    let strategies = StrategyService::new(conn.clone(), clock.clone());
    strategies.watch_users(&users);
    strategies.watch_groups(&groups);
    let books = AddressBookService::new(conn.clone(), settings.clone(), crypto.clone());
    let devices = DeviceService::new(
        conn.clone(),
        settings.clone(),
        crypto,
        strategies.clone(),
        clock.clone(),
    );
    Ctx {
        conn: conn.clone(),
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

    /// Moves the clock past the per-device sysinfo write interval.
    pub fn next_write_window(&self) {
        self.clock.advance(SYSINFO_WRITE_INTERVAL);
    }

    /// Sets `devices.accept_presets`.
    pub async fn accept_presets(&self, mode: &str) {
        self.set("devices.accept_presets", serde_json::json!(mode))
            .await;
    }

    /// User `alice`, device group `Office`, strategy `Locked`, admin-managed shared book
    /// `Team`: what [`all_presets`](super::presets::all_presets) names.
    pub async fn targets(&self) -> Targets {
        Targets {
            alice: self.user("alice").await.id,
            group: self.devices.create_group("Office", "").await.unwrap(),
            strategy: self.strategy("Locked").await.id,
            book: self.shared_book("Team").await.guid,
        }
    }

    /// Grants `user` `rule` on book `guid`.
    pub async fn grant(&self, guid: &str, user: i64, rule: Rule) {
        self.books
            .set_share(Actor::Admin, guid, Subject::User(user), rule)
            .await
            .unwrap();
    }

    pub async fn book_peers(&self, guid: &str) -> Vec<Peer> {
        let page = self.books.peers(Actor::Admin, guid, PageRequest::default());
        page.await.unwrap().items
    }

    /// Owner, device group and note of device [`ID`], set directly (the console's job).
    pub async fn preassign(&self, owner: i64, group: i64) {
        let mut row = self.row(ID).await.into_active_model();
        row.owner_user_id = Set(Some(owner));
        row.device_group_id = Set(Some(group));
        row.note = Set(Some("set by admin".to_owned()));
        row.update(&self.conn).await.unwrap();
    }

    /// Fills book `guid` up to its tag limit.
    pub async fn fill_tags(&self, guid: &str) {
        let book_id = self.books.find_shared_by_name("Tagged").await.unwrap().id;
        assert_eq!(
            self.books.find_shared_by_name("Tagged").await.unwrap().guid,
            guid
        );
        let rows = (0..MAX_BOOK_TAGS).map(|i| address_book_tag::ActiveModel {
            address_book_id: Set(book_id),
            name: Set(format!("tag-{i}")),
            color: Set(0),
        });
        address_book_tag::Entity::insert_many(rows)
            .exec(&self.conn)
            .await
            .unwrap();
    }
}

/// The rows [`Ctx::targets`] creates.
pub struct Targets {
    pub alice: i64,
    pub group: i64,
    pub strategy: i64,
    pub book: String,
}

/// `addressbooks::tags::MAX_BOOK_TAGS`.
const MAX_BOOK_TAGS: u64 = 1000;
