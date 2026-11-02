//! Fixture `seed` names → functions that prepare the fresh app before the
//! request is replayed (users, sessions, devices, settings, ...).
//!
//! Seeds write through the real services/database of the app's one
//! [`AppState`] (the instance the routers serve). Each returns named outputs
//! (e.g. `session_token` → the token the real service issued) that the
//! fixture's request refers to as `${session_token}` (see
//! [`crate::fixture::Request::resolve`]). Prefer fixed, documented values
//! where the service allows it. Keep each seed small and named after what it
//! creates.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use anyhow::Context;
use deskhaven_config::settings::{AUTH_LOCKOUT_MAX_FAILURES, Change};
use deskhaven_core::auth::limiter::ClientAddr;
use deskhaven_core::clock::{Clock, SystemClock};
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::settings::UpdateRequest;
use deskhaven_core::state::AppState;
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::users::{Credential, NewUser, Password, Status};

use crate::app::DEFAULT_PEER;

/// Output name → value, substituted into `${name}` placeholders.
pub type SeedOutputs = BTreeMap<String, String>;
pub type SeedFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<SeedOutputs>> + Send + 'a>>;
pub type Seed = for<'a> fn(&'a AppState) -> SeedFuture<'a>;

/// Register as `("name", |state| Box::pin(my_seed(state)))`.
const BUILTIN: &[(&str, Seed)] = &[
    ("client-session", |state| Box::pin(client_session(state))),
    ("console-session", |state| Box::pin(console_session(state))),
    ("local-user", |state| Box::pin(local_user(state))),
    ("disabled-user", |state| Box::pin(disabled_user(state))),
    ("unverified-user", |state| Box::pin(unverified_user(state))),
    ("login-locked-out", |state| {
        Box::pin(login_locked_out(state))
    }),
    ("totp-user", |state| Box::pin(totp_user(state))),
    ("tfa-enforce-all", |state| Box::pin(tfa_enforce_all(state))),
];

/// `client-session` / `local-user` values (fixed, so fixtures may also spell
/// them out). Every seeded account has [`SEED_PASSWORD`].
pub const SEED_USERNAME: &str = "alice";
pub const SEED_PASSWORD: &str = "contract-Seed-pass-1";
pub const SEED_RUSTDESK_ID: &str = "123456789";
pub const SEED_UUID: &str = "contract-seed-device-uuid";
/// `disabled-user`.
pub const DISABLED_USERNAME: &str = "dave";
/// `unverified-user`.
pub const UNVERIFIED_USERNAME: &str = "erin";
/// `console-session`.
pub const CONSOLE_USERNAME: &str = "carol";
/// A username no seed creates.
pub const UNKNOWN_USERNAME: &str = "nobody";

async fn create_user(state: &AppState, username: &str, status: Status) -> anyhow::Result<i64> {
    let password = Password::new(SEED_PASSWORD);
    let new = NewUser {
        status,
        ..NewUser::new(username, Credential::Local { password })
    };
    Ok(state.users.create(new).await?.id)
}

/// An active local user [`SEED_USERNAME`] with [`SEED_PASSWORD`], not logged
/// in. No outputs.
async fn local_user(state: &AppState) -> anyhow::Result<SeedOutputs> {
    create_user(state, SEED_USERNAME, Status::Active).await?;
    Ok(SeedOutputs::new())
}

/// A disabled local user [`DISABLED_USERNAME`]. No outputs.
async fn disabled_user(state: &AppState) -> anyhow::Result<SeedOutputs> {
    create_user(state, DISABLED_USERNAME, Status::Disabled).await?;
    Ok(SeedOutputs::new())
}

/// An unverified local user [`UNVERIFIED_USERNAME`]. No outputs.
async fn unverified_user(state: &AppState) -> anyhow::Result<SeedOutputs> {
    create_user(state, UNVERIFIED_USERNAME, Status::Unverified).await?;
    Ok(SeedOutputs::new())
}

/// An active user [`CONSOLE_USERNAME`] with a **console** session. Outputs
/// `console_token`.
async fn console_session(state: &AppState) -> anyhow::Result<SeedOutputs> {
    create_user(state, CONSOLE_USERNAME, Status::Active).await?;
    let login = state
        .users
        .authenticate_local(CONSOLE_USERNAME, &Password::new(SEED_PASSWORD))
        .await?;
    let (_, token) = state
        .sessions
        .create_console(login.user.id, &login.stamp, None, None)
        .await?;
    Ok(SeedOutputs::from([(
        "console_token".to_owned(),
        token.expose().to_owned(),
    )]))
}

/// Spends the whole `auth.lockout.max_failures` budget of [`SEED_USERNAME`]
/// and of [`UNKNOWN_USERNAME`] from the default peer through the real login
/// throttle (failed attempts), so the next login of either from that peer is
/// locked out. Creates no user. No outputs.
async fn login_locked_out(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let client = ClientAddr {
        addr: DEFAULT_PEER.ip(),
        is_proxy_fallback: false,
    };
    let budget = state.settings.snapshot().get(&AUTH_LOCKOUT_MAX_FAILURES);
    for name in [SEED_USERNAME, UNKNOWN_USERNAME] {
        for _ in 0..budget {
            let attempt = state.login_throttle.pre_check(client, name).await?;
            let _ = state.login_throttle.on_failure(attempt);
        }
    }
    Ok(SeedOutputs::new())
}

/// An active local user [`SEED_USERNAME`] with TOTP enabled through the real
/// 2FA service. The enrolment is confirmed with the code of the **previous**
/// time step, so the authenticator's current and next codes both still log in.
/// Outputs `totp_secret` (base32 seed) and `recovery_code` (one of its codes).
async fn totp_user(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let user_id = create_user(state, SEED_USERNAME, Status::Active).await?;
    let enrolment = state.tfa.begin_enrolment(user_id).await?;
    let secret =
        TotpSecret::from_base32(&enrolment.secret).context("enrolment seed is not base32")?;
    let previous = totp::step_at(SystemClock.now_ms()) - 1;
    let codes = state
        .tfa
        .confirm_enrolment(user_id, &totp::code_at_step(&secret, previous))
        .await?;
    let recovery = codes.codes().next().context("no recovery code issued")?;
    Ok(SeedOutputs::from([
        ("totp_secret".to_owned(), enrolment.secret.to_string()),
        ("recovery_code".to_owned(), recovery.to_owned()),
    ]))
}

/// Sets `auth.tfa.enforce` to `all` through the settings service. No outputs.
async fn tfa_enforce_all(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let key = "auth.tfa.enforce";
    let settings = &state.settings;
    let version = settings
        .snapshot()
        .view(key)
        .with_context(|| format!("unknown setting {key:?}"))?
        .version;
    let change = Change {
        key: key.to_owned(),
        value: serde_json::json!("all"),
        version,
    };
    settings
        .update(UpdateRequest {
            changes: vec![change],
            actor: None,
            reason: None,
        })
        .await?;
    Ok(SeedOutputs::new())
}

/// An active local user [`SEED_USERNAME`] logged in from device
/// [`SEED_RUSTDESK_ID`] / [`SEED_UUID`] through the real user and session
/// services. Outputs `session_token` (the bearer token the service issued),
/// `username`, `rustdesk_id` and `uuid`.
async fn client_session(state: &AppState) -> anyhow::Result<SeedOutputs> {
    create_user(state, SEED_USERNAME, Status::Active).await?;
    let login = state
        .users
        .authenticate_local(SEED_USERNAME, &Password::new(SEED_PASSWORD))
        .await?;
    let device = ClientDevice {
        rustdesk_id: SEED_RUSTDESK_ID.to_owned(),
        uuid: SEED_UUID.to_owned(),
        info: Some(serde_json::json!({ "os": "linux", "type": "client", "name": "seed" })),
    };
    let (_, token) = state
        .sessions
        .create_client(login.user.id, &login.stamp, &device, None)
        .await?;
    Ok(SeedOutputs::from([
        ("session_token".to_owned(), token.expose().to_owned()),
        ("username".to_owned(), SEED_USERNAME.to_owned()),
        ("rustdesk_id".to_owned(), SEED_RUSTDESK_ID.to_owned()),
        ("uuid".to_owned(), SEED_UUID.to_owned()),
    ]))
}

/// A seed registry: the built-ins, plus test-only seeds of runner tests.
#[derive(Clone)]
pub struct Seeds {
    entries: Vec<(&'static str, Seed)>,
}

impl Default for Seeds {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Seeds {
    pub fn builtin() -> Self {
        Self {
            entries: BUILTIN.to_vec(),
        }
    }

    /// The registry plus `name`. The first registration of a name wins, so
    /// a test seed cannot shadow a built-in.
    #[must_use]
    pub fn with(self, name: &'static str, seed: Seed) -> Self {
        let entries = self.entries.into_iter().chain([(name, seed)]).collect();
        Self { entries }
    }

    pub fn lookup(&self, name: &str) -> Option<Seed> {
        self.entries
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, seed)| *seed)
    }

    /// Runs `names` in order against `state` and collects their outputs. An
    /// output name produced twice is an error (a placeholder must have one
    /// meaning).
    pub async fn apply(&self, names: &[String], state: &AppState) -> anyhow::Result<SeedOutputs> {
        let mut outputs = SeedOutputs::new();
        for name in names {
            let seed = self
                .lookup(name)
                .with_context(|| format!("unknown seed {name:?}"))?;
            let produced = seed(state)
                .await
                .with_context(|| format!("seed {name:?} failed"))?;
            for (key, value) in produced {
                anyhow::ensure!(
                    !outputs.contains_key(&key),
                    "seed output {key:?} is defined twice (last by seed {name:?})"
                );
                outputs.insert(key, value);
            }
        }
        Ok(outputs)
    }
}
