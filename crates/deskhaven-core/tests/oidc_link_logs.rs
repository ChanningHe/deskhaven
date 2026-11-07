//! What a refused link logs (M4-09): a warning naming the account and the provider, never
//! the IdP subject. Its own test binary: the capture is a thread-local subscriber. Both
//! backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::capture_logs;
use common::oidc::{Env, PROVIDER};
use deskhaven_core::oidc::LinkFailure;
use deskhaven_db::testing::{TestDb, each_migrated_backend};

#[tokio::test]
async fn a_refused_link_is_logged_without_the_subject() {
    let logs = capture_logs();
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let bob = env.local_user("bob", None).await;
        env.link(alice.id, "secret-sub-1", |s| s)
            .await
            .outcome
            .unwrap();
        logs.take();

        let report = env.link(bob.id, "secret-sub-1", |s| s).await;

        let b = db.backend;
        let failure = report.outcome.as_ref().err().copied();
        assert_eq!(failure, Some(LinkFailure::IdentityTaken), "{b}");
        let lines = logs.take();
        let line = lines
            .lines()
            .find(|l| l.contains("WARN") && l.contains("identity_taken"))
            .unwrap_or_else(|| panic!("{b}: no warning in {lines}"));
        assert!(line.contains(&format!("user_id={}", bob.id)), "{b}: {line}");
        assert!(line.contains(PROVIDER), "{b}: {line}");
        assert!(!lines.contains("secret-sub-1"), "{b}: {lines}");
    })
    .await;
}
