//! Shared setup for the address book tests.

use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::addressbooks::{
    Actor, AddressBookError, AddressBookService, Book, NewSharedBook, PeerPatch, Rule, Subject,
};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::groups::{GroupService, NewGroup};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, UserService,
};
use deskhaven_db::testing::TestDb;
use serde_json::Value;

const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

/// Every service of one test database, sharing one settings service and master key.
pub struct Env {
    pub books: AddressBookService,
    pub users: UserService,
    pub groups: GroupService,
    pub settings: SettingsService,
    pub crypto: Arc<SecretBox>,
}

pub async fn env(db: &TestDb) -> Env {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    let (settings, _) =
        SettingsService::load(db.conn.clone(), Arc::clone(&crypto), PinSet::empty())
            .await
            .unwrap();
    let users = UserService::new(
        db.conn.clone(),
        settings.clone(),
        PasswordHasher::new(FAST).unwrap(),
    );
    Env {
        books: AddressBookService::new(db.conn.clone(), settings.clone(), Arc::clone(&crypto)),
        groups: GroupService::new(db.conn.clone()),
        users,
        settings,
        crypto,
    }
}

impl Env {
    pub async fn user(&self, name: &str) -> i64 {
        let credential = Credential::Local {
            password: Password::new("tundra-Lamp-42"),
        };
        self.users
            .create(NewUser::new(name, credential))
            .await
            .unwrap()
            .id
    }

    pub async fn group(&self, name: &str, members: &[i64]) -> i64 {
        let group = self.groups.create(NewGroup::new(name)).await.unwrap();
        for &member in members {
            self.groups.add_member(group.id, member).await.unwrap();
        }
        group.id
    }

    /// A shared book named `name`, owned by `owner` (`None`: admin-managed).
    pub async fn shared(&self, name: &str, owner: Option<i64>) -> Book {
        let new = NewSharedBook {
            name: name.to_owned(),
            note: String::new(),
            owner_user_id: owner,
        };
        self.books.create_shared(new).await.unwrap()
    }

    /// A shared book granted to `user` with `rule` by the administrator.
    pub async fn shared_with(&self, name: &str, user: i64, rule: Rule) -> Book {
        let book = self.shared(name, None).await;
        self.books
            .set_share(Actor::Admin, &book.guid, Subject::User(user), rule)
            .await
            .unwrap();
        book
    }

    /// Ids of the first page (100) of peers `actor` reads in `guid`.
    pub async fn peer_ids(&self, actor: Actor, guid: &str) -> Vec<String> {
        let page = self.books.peers(actor, guid, page(1, 100)).await.unwrap();
        page.items.into_iter().map(|p| p.id).collect()
    }
}

pub fn page(page: u64, page_size: u64) -> PageRequest {
    PageRequest { page, page_size }
}

/// A peer as the client sends it.
pub fn peer(value: Value) -> PeerPatch {
    PeerPatch::from_json(value).unwrap()
}

/// `id` with nothing else.
pub fn bare(id: &str) -> PeerPatch {
    peer(serde_json::json!({ "id": id }))
}

pub fn is_not_found<T: std::fmt::Debug>(result: Result<T, AddressBookError>) -> bool {
    matches!(result, Err(AddressBookError::NotFound))
}

pub fn is_forbidden<T: std::fmt::Debug>(result: Result<T, AddressBookError>) -> bool {
    matches!(result, Err(AddressBookError::Forbidden))
}
