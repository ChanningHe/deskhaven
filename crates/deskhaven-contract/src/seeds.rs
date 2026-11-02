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
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Credential, NewUser, Password};

/// Output name → value, substituted into `${name}` placeholders.
pub type SeedOutputs = BTreeMap<String, String>;
pub type SeedFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<SeedOutputs>> + Send + 'a>>;
pub type Seed = for<'a> fn(&'a AppState) -> SeedFuture<'a>;

/// Register as `("name", |state| Box::pin(my_seed(state)))`.
const BUILTIN: &[(&str, Seed)] = &[("client-session", |state| Box::pin(client_session(state)))];

/// `client-session` values (fixed, so fixtures may also spell them out).
pub const SEED_USERNAME: &str = "alice";
pub const SEED_PASSWORD: &str = "contract-Seed-pass-1";
pub const SEED_RUSTDESK_ID: &str = "123456789";
pub const SEED_UUID: &str = "contract-seed-device-uuid";

/// An active local user [`SEED_USERNAME`] logged in from device
/// [`SEED_RUSTDESK_ID`] / [`SEED_UUID`] through the real user and session
/// services. Outputs `session_token` (the bearer token the service issued),
/// `username`, `rustdesk_id` and `uuid`.
async fn client_session(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let password = Password::new(SEED_PASSWORD);
    state
        .users
        .create(NewUser::new(SEED_USERNAME, Credential::Local { password }))
        .await?;
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
