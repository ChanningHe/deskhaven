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
use deskhaven_core::state::AppState;

/// Output name → value, substituted into `${name}` placeholders.
pub type SeedOutputs = BTreeMap<String, String>;
pub type SeedFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<SeedOutputs>> + Send + 'a>>;
pub type Seed = for<'a> fn(&'a AppState) -> SeedFuture<'a>;

/// Empty until an endpoint needs state (M2-07 adds users and sessions).
/// Register as `("name", |state| Box::pin(my_seed(state)))`.
const BUILTIN: &[(&str, Seed)] = &[];

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
