//! OIDC service tests: the real service on the test database with a manual clock, a
//! provider pointing at the mock IdP, and one-call logins.

use std::sync::Arc;

use deskhaven_core::clock::{Clock, ManualClock};
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::oidc::{
    CallbackParams, CallbackReport, ClientFlowStart, NewProvider, OidcService, Provider,
    ReqwestFetch,
};
use deskhaven_core::sessions::{ClientDevice, SessionService};
use deskhaven_core::settings::SettingsService;
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, User, UserService,
};
use deskhaven_db::testing::TestDb;
use serde_json::json;

use super::oidc_idp::{AUTH_CODE, AuthRequest, CLIENT_ID, CLIENT_SECRET, MockIdp, TokenSpec};
use super::{clock, settings};

pub const PUBLIC_URL: &str = "https://rd.example.com";
pub const RUSTDESK_ID: &str = "123456789";
pub const UUID: &str = "b2lkYy10ZXN0LXV1aWQ=";
pub const PROVIDER: &str = "corp";
pub const PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

pub struct Env {
    pub oidc: OidcService,
    pub users: UserService,
    pub sessions: SessionService,
    pub settings: SettingsService,
    pub clock: Arc<ManualClock>,
    pub idp: MockIdp,
    pub provider: Provider,
}

pub fn device() -> ClientDevice {
    ClientDevice {
        rustdesk_id: RUSTDESK_ID.to_owned(),
        uuid: UUID.to_owned(),
        info: Some(json!({"os": "linux", "type": "client", "name": "oidc-host"})),
    }
}

impl Env {
    /// A confidential provider `corp` without auto-provisioning.
    pub async fn new(db: &TestDb) -> Self {
        Self::with(db, |_| {}).await
    }

    /// `corp` adjusted by `adjust` before creation.
    pub async fn with(db: &TestDb, adjust: impl FnOnce(&mut NewProvider)) -> Self {
        let settings = settings(db).await;
        let clock = clock();
        let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
        let hasher = PasswordHasher::new(FAST).unwrap();
        let users =
            UserService::with_clock(db.conn.clone(), settings.clone(), hasher, clock.clone());
        let sessions = SessionService::new(
            db.conn.clone(),
            settings.clone(),
            users.clone(),
            clock.clone(),
        );
        let oidc = OidcService::new(
            db.conn.clone(),
            crypto,
            settings.clone(),
            users.clone(),
            clock.clone(),
            PUBLIC_URL,
            Arc::new(ReqwestFetch::new()),
        );
        let idp = MockIdp::start().await;
        let mut new = NewProvider::new(PROVIDER, idp.issuer.clone(), CLIENT_ID);
        new.client_secret = Some(CLIENT_SECRET.to_owned());
        adjust(&mut new);
        let provider = oidc.create_provider(new).await.unwrap();
        Self {
            oidc,
            users,
            sessions,
            settings,
            clock,
            idp,
            provider,
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.clock.now_ms()
    }

    pub async fn begin(&self) -> (ClientFlowStart, AuthRequest) {
        let start = self.oidc.begin_client(PROVIDER, &device()).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        (start, request)
    }

    /// A valid token spec for `request`'s nonce.
    pub fn spec(&self, subject: &str, request: &AuthRequest) -> TokenSpec {
        TokenSpec::for_flow(subject, &request.nonce, self.now_ms())
    }

    /// The IdP redirect for `request` with the mock's code.
    pub async fn callback(&self, request: &AuthRequest) -> CallbackReport {
        let params = CallbackParams {
            state: Some(&request.state),
            code: Some(AUTH_CODE),
            error: None,
        };
        self.oidc.complete_callback(&params).await.unwrap()
    }

    /// Begin, a token per `shape(spec)`, callback.
    pub async fn login(
        &self,
        subject: &str,
        shape: impl FnOnce(TokenSpec) -> TokenSpec,
    ) -> (ClientFlowStart, CallbackReport) {
        let (start, request) = self.begin().await;
        self.idp
            .mount_token(&shape(self.spec(subject, &request)))
            .await;
        let report = self.callback(&request).await;
        (start, report)
    }

    /// A local account with [`PASSWORD`] (and `email`, if given).
    pub async fn local_user(&self, username: &str, email: Option<&str>) -> User {
        let new = NewUser {
            email: email.map(str::to_owned),
            email_verified: email.is_some(),
            ..NewUser::new(
                username,
                Credential::Local {
                    password: Password::new(PASSWORD),
                },
            )
        };
        self.users.create(new).await.unwrap()
    }
}
