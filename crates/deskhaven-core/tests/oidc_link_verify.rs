//! Verifying a link flow (M4-09) under concurrency and failure: two verifications at once
//! exchange the code once (the loser does not ask the IdP and is told to come back), a
//! verification that never finished does not wedge the flow past a short grace period, and
//! a provider disabled between verification and confirmation refuses the link. Both
//! backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::oidc::{Env, PROVIDER};
use common::oidc_idp::{AUTH_CODE, AuthRequest};
use deskhaven_core::oidc::{CallbackError, CallbackParams, LinkFailure, ProviderUpdate};
use deskhaven_db::entity::oidc_flow;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};
use serde_json::{Value, json};

fn params(request: &AuthRequest) -> CallbackParams<'_> {
    CallbackParams {
        state: Some(&request.state),
        code: Some(AUTH_CODE),
        error: None,
    }
}

#[tokio::test]
async fn concurrent_verifications_exchange_the_code_once() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-a", &request)).await;
        let binding = Some(start.binding.expose());
        let answer = params(&request);

        let (one, two) = tokio::join!(
            env.oidc.verify_link(&answer, binding, alice.id),
            env.oidc.verify_link(&answer, binding, alice.id),
        );
        let after = env.oidc.verify_link(&answer, binding, alice.id).await;

        let b = db.backend;
        for result in [&one, &two] {
            assert!(
                matches!(result, Ok(Ok(_)) | Err(CallbackError::Verifying)),
                "{b}: {result:?}"
            );
        }
        assert!(matches!(one, Ok(Ok(_))) || matches!(two, Ok(Ok(_))), "{b}");
        assert_eq!(env.idp.token_requests().await.len(), 1, "{b}: one exchange");
        assert_eq!(
            after.unwrap().unwrap().subject,
            "sub-a",
            "{b}: ends verified"
        );
    })
    .await;
}

/// A verification that claimed the flow and never stored an outcome (a crash mid-exchange)
/// blocks others for a short grace period only.
#[tokio::test]
async fn an_unfinished_verification_does_not_wedge_the_flow() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-a", &request)).await;
        let binding = Some(start.binding.expose());
        let row = oidc_flow::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        let mut context: Value = serde_json::from_str(&row.context).unwrap();
        context["verifying_at"] = json!(env.now_ms());
        let mut claimed: oidc_flow::ActiveModel = row.into();
        claimed.context = Set(context.to_string());
        claimed.update(&db.conn).await.unwrap();

        let blocked = env
            .oidc
            .verify_link(&params(&request), binding, alice.id)
            .await;
        env.clock.advance(Duration::from_secs(61));
        let later = env
            .oidc
            .verify_link(&params(&request), binding, alice.id)
            .await;

        let b = db.backend;
        assert!(
            matches!(blocked, Err(CallbackError::Verifying)),
            "{b}: {blocked:?}"
        );
        assert_eq!(later.unwrap().unwrap().subject, "sub-a", "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_provider_disabled_after_verification_refuses_the_link() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", None).await;
        let start = env.oidc.begin_link(PROVIDER, alice.id).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-a", &request)).await;
        let binding = Some(start.binding.expose());
        env.oidc
            .verify_link(&params(&request), binding, alice.id)
            .await
            .unwrap()
            .unwrap();
        let off = ProviderUpdate {
            enabled: Some(false),
            ..ProviderUpdate::default()
        };
        env.oidc
            .update_provider(env.provider.id, off)
            .await
            .unwrap();

        let guard = env.link_guard(alice.id, "user.identity_link");
        let report = env
            .oidc
            .complete_link(Some(&request.state), binding, alice.id, guard)
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(report.outcome.err(), Some(LinkFailure::Unavailable), "{b}");
        assert!(
            env.oidc.identities_of(alice.id).await.unwrap().is_empty(),
            "{b}"
        );
    })
    .await;
}
