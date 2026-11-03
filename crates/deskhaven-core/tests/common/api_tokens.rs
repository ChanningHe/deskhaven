//! Shared setup for the API token tests: a token service and a user service on one manual
//! clock.

use std::sync::Arc;

use deskhaven_config::settings::PinSet;
use deskhaven_core::api_tokens::{ApiTokenService, NewApiToken, Scope};
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, Role, UserService,
};
use deskhaven_db::entity::api_token;
use deskhaven_db::testing::TestDb;
use sea_orm::{DatabaseConnection, EntityTrait};
use sha2::{Digest, Sha256};

pub const T0_MS: i64 = 1_700_000_000_123;
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

pub struct Env {
    pub users: UserService,
    pub tokens: ApiTokenService,
    pub clock: Arc<ManualClock>,
    pub db: DatabaseConnection,
}

impl Env {
    pub async fn new(db: &TestDb) -> Self {
        let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
        let (settings, _) = SettingsService::load(db.conn.clone(), crypto, PinSet::empty())
            .await
            .unwrap();
        let clock = Arc::new(ManualClock::at(T0_MS));
        let hasher = PasswordHasher::new(FAST).unwrap();
        let users = UserService::with_clock(db.conn.clone(), settings, hasher, clock.clone());
        let tokens = ApiTokenService::new(db.conn.clone(), clock.clone());
        Self {
            users,
            tokens,
            clock,
            db: db.conn.clone(),
        }
    }

    pub async fn admin(&self, name: &str) -> i64 {
        let password = Password::new("tundra-Lamp-42");
        let new = NewUser {
            role: Role::Admin,
            ..NewUser::new(name, Credential::Local { password })
        };
        self.users.create(new).await.unwrap().id
    }

    pub fn new_token(&self, created_by: Option<i64>, scopes: &[Scope]) -> NewApiToken {
        NewApiToken {
            name: "deploy script".to_owned(),
            scopes: scopes.iter().copied().collect(),
            created_by,
            expires_at: None,
        }
    }

    pub async fn issue(&self, created_by: Option<i64>, scopes: &[Scope]) -> (i64, String) {
        let (token, secret) = self
            .tokens
            .create(self.new_token(created_by, scopes))
            .await
            .unwrap();
        (token.id, secret.expose().to_owned())
    }

    pub async fn row(&self, id: i64) -> api_token::Model {
        api_token::Entity::find_by_id(id)
            .one(&self.db)
            .await
            .unwrap()
            .unwrap()
    }
}

pub fn sha256_hex(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
