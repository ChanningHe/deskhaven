//! Helpers for the 2FA service tests: a user service with a fast hasher, a manual clock
//! shared with the 2FA service, and an authenticator emulation.

use std::sync::Arc;

use deskhaven_core::auth::limiter::PendingSubject;
use deskhaven_core::clock::ManualClock;
use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::settings::SettingsService;
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::tfa::{
    ChallengeError, ChallengeSecret, Passed, PendingLogin, RecoveryCodes, SecondStep, TfaService,
};
use deskhaven_core::users::{
    Credential, CredentialStamp, HashParams, NewUser, Password, PasswordHasher, Role, UserService,
};
use deskhaven_db::testing::TestDb;

use super::{clock, settings};

pub const PASSWORD: &str = "tundra-Lamp-42";
pub const RUSTDESK_ID: &str = "123456789";
pub const UUID: &str = "dGZhLXRlc3QtdXVpZA==";
pub const SOURCE: &str = "192.0.2.1";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

pub struct Env {
    pub settings: SettingsService,
    pub users: UserService,
    pub tfa: TfaService,
    pub crypto: Arc<SecretBox>,
    pub clock: Arc<ManualClock>,
}

/// An enrolled user: its id, password stamp and authenticator seed.
pub struct Enrolled {
    pub user_id: i64,
    pub username: String,
    pub stamp: CredentialStamp,
    pub secret: TotpSecret,
    pub recovery: RecoveryCodes,
}

impl Env {
    pub async fn new(db: &TestDb) -> Self {
        let settings = settings(db).await;
        let clock = clock();
        let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
        let users = UserService::with_clock(
            db.conn.clone(),
            settings.clone(),
            PasswordHasher::new(FAST).unwrap(),
            clock.clone(),
        );
        let tfa = TfaService::new(
            db.conn.clone(),
            Arc::clone(&crypto),
            settings.clone(),
            clock.clone(),
        );
        Self {
            settings,
            users,
            tfa,
            crypto,
            clock,
        }
    }

    pub fn now_ms(&self) -> i64 {
        use deskhaven_core::clock::Clock;
        self.clock.now_ms()
    }

    /// A local account with [`PASSWORD`] and the stamp of a successful login.
    pub async fn user(&self, username: &str, role: Role) -> (i64, CredentialStamp) {
        let password = Password::new(PASSWORD);
        let new = NewUser {
            role,
            ..NewUser::new(username, Credential::Local { password })
        };
        self.users.create(new).await.unwrap();
        let login = self
            .users
            .authenticate_local(username, &Password::new(PASSWORD))
            .await
            .unwrap();
        (login.user.id, login.stamp)
    }

    /// A user with TOTP enabled (the confirming step is the current one).
    pub async fn enrolled(&self, username: &str) -> Enrolled {
        let (user_id, _) = self.user(username, Role::User).await;
        let enrolment = self.tfa.begin_enrolment(user_id).await.unwrap();
        let secret = TotpSecret::from_base32(&enrolment.secret).unwrap();
        let recovery = self
            .tfa
            .confirm_enrolment(user_id, &totp::code_at(&secret, self.now_ms()))
            .await
            .unwrap();
        // Enabling 2FA does not change the password stamp.
        let stamp = self
            .users
            .authenticate_local(username, &Password::new(PASSWORD))
            .await
            .unwrap()
            .stamp;
        Enrolled {
            user_id,
            username: username.to_owned(),
            stamp,
            secret,
            recovery,
        }
    }

    pub async fn challenge(&self, user: &Enrolled) -> ChallengeSecret {
        let lockout = pending(&user.username);
        let login = PendingLogin {
            user_id: user.user_id,
            stamp: &user.stamp,
            device: &device(),
            lockout: &lockout,
        };
        self.tfa.issue_challenge(&login).await.unwrap()
    }

    pub async fn verify(
        &self,
        secret: &ChallengeSecret,
        username: &str,
        code: &str,
    ) -> Result<Passed, ChallengeError> {
        self.verify_from(secret, username, &device(), code).await
    }

    pub async fn verify_from(
        &self,
        secret: &ChallengeSecret,
        username: &str,
        device: &ClientDevice,
        code: &str,
    ) -> Result<Passed, ChallengeError> {
        let step = SecondStep {
            secret: secret.expose(),
            username,
            device,
            code,
        };
        self.tfa.verify_challenge(&step).await
    }

    /// The authenticator's current code.
    pub fn code(&self, user: &Enrolled) -> String {
        totp::code_at(&user.secret, self.now_ms())
    }

    /// The code `delta` steps from now.
    pub fn code_in(&self, user: &Enrolled, delta: i64) -> String {
        totp::code_at_step(&user.secret, totp::step_at(self.now_ms()) + delta)
    }
}

pub fn device() -> ClientDevice {
    ClientDevice {
        rustdesk_id: RUSTDESK_ID.to_owned(),
        uuid: UUID.to_owned(),
        info: None,
    }
}

pub fn pending(username: &str) -> PendingSubject {
    PendingSubject::from_parts(&username.to_ascii_lowercase(), SOURCE).unwrap()
}

pub fn first_code(codes: &RecoveryCodes) -> String {
    codes.codes().next().unwrap().to_owned()
}
