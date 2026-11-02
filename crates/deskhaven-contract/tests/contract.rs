//! Replays every fixture in `tests/contract/fixtures` through the real
//! client API. `CONTRACT_FILTER=login-options/` runs a subset;
//! `-- --nocapture` lists each fixture.
#![allow(clippy::panic)]

use deskhaven_contract::runner::{contract_dir, run_dir};

#[tokio::test]
async fn client_api_fixtures() {
    let filter = std::env::var("CONTRACT_FILTER").ok();

    let outcome = match run_dir(&contract_dir(), filter.as_deref()).await {
        Ok(outcome) => outcome,
        Err(err) => panic!("cannot run the contract fixtures: {err:#}"),
    };

    println!("{outcome}");
    assert!(outcome.failures.is_empty(), "\n{outcome}");
    assert!(
        !outcome.passed.is_empty(),
        "no fixture matched (filter: {filter:?})"
    );
}
