//! Fixture `seed` names → functions that prepare the fresh app before the
//! request is replayed (users, sessions, devices, settings, ...).
//!
//! Seeds write through the real services/database of [`AppState`] and use
//! fixed, documented values (e.g. a session whose token is a known constant)
//! so a fixture can refer to them literally (`"Authorization": "Bearer
//! <that constant>"`). Keep each seed small and named after what it creates.

use std::future::Future;
use std::pin::Pin;

use deskhaven_core::state::AppState;

pub type SeedFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
pub type Seed = for<'a> fn(&'a AppState) -> SeedFuture<'a>;

/// Empty until an endpoint needs state (M2-07 adds users and sessions).
/// Register as `("name", |state| Box::pin(my_seed(state)))`.
const SEEDS: &[(&str, Seed)] = &[];

pub fn lookup(name: &str) -> Option<Seed> {
    SEEDS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, seed)| *seed)
}
