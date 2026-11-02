//! Console OIDC logins (M3-02) against the mock IdP: the flow completes only with the
//! browser binding of `begin_console`, a wrong or missing binding leaves it untouched,
//! the client completion refuses console flows (and vice versa), and the row is gone once
//! the flow has an outcome. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, PROVIDER};
use common::oidc_idp::{AUTH_CODE, AuthRequest};
use deskhaven_core::oidc::{CallbackError, CallbackParams, Failure, FlowKind};
use deskhaven_db::entity::oidc_flow;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};

fn params(request: &AuthRequest) -> CallbackParams<'_> {
    CallbackParams {
        state: Some(&request.state),
        code: Some(AUTH_CODE),
        error: None,
    }
}

async fn provisioning(db: &TestDb) -> Env {
    Env::with(db, |p| p.auto_provision = true).await
}

async fn flows(db: &TestDb) -> u64 {
    oidc_flow::Entity::find().count(&db.conn).await.unwrap()
}

#[tokio::test]
async fn a_console_flow_completes_with_its_binding_and_is_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let env = provisioning(db).await;
        let start = env.oidc.begin_console(PROVIDER).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-1", &request)).await;
        let b = db.backend;

        let report = env
            .oidc
            .complete_console(&params(&request), Some(start.binding.expose()))
            .await
            .unwrap();

        assert_eq!(report.flow, FlowKind::Console, "{b}");
        let login = report.outcome.unwrap();
        assert!(
            env.sessions
                .create_console(login.user.id, &login.stamp, None, None)
                .await
                .is_ok(),
            "{b}: the stamp mints a console session"
        );
        assert_eq!(flows(db).await, 0, "{b}: row deleted");
    })
    .await;
}

#[tokio::test]
async fn a_wrong_or_missing_binding_or_the_client_path_leaves_the_flow_untouched() {
    each_migrated_backend(async |db: &TestDb| {
        let env = provisioning(db).await;
        let start = env.oidc.begin_console(PROVIDER).await.unwrap();
        let other = env.oidc.begin_console(PROVIDER).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        env.idp.mount_token(&env.spec("sub-1", &request)).await;
        let b = db.backend;

        let missing = env.oidc.complete_console(&params(&request), None).await;
        let wrong = env
            .oidc
            .complete_console(&params(&request), Some(other.binding.expose()))
            .await;
        let malformed = env
            .oidc
            .complete_console(&params(&request), Some("cookie"))
            .await;
        let as_client = env.oidc.complete_callback(&params(&request)).await;

        for (name, result) in [
            ("missing", missing),
            ("wrong", wrong),
            ("malformed", malformed),
            ("client", as_client),
        ] {
            assert!(
                matches!(result, Err(CallbackError::InvalidState)),
                "{b}: {name}: {result:?}"
            );
        }
        assert!(
            env.idp.token_requests().await.is_empty(),
            "{b}: no exchange"
        );
        let done = env
            .oidc
            .complete_console(&params(&request), Some(start.binding.expose()))
            .await
            .unwrap();
        assert!(done.outcome.is_ok(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_client_flow_never_completes_through_the_console() {
    each_migrated_backend(async |db: &TestDb| {
        let env = provisioning(db).await;
        let console = env.oidc.begin_console(PROVIDER).await.unwrap();
        let (_, request) = env.begin().await;
        let b = db.backend;

        let result = env
            .oidc
            .complete_console(&params(&request), Some(console.binding.expose()))
            .await;

        assert!(matches!(result, Err(CallbackError::InvalidState)), "{b}");
        let preview = env.oidc.preview_callback(Some(&request.state)).await;
        assert!(preview.is_ok(), "{b}: still live");
    })
    .await;
}

#[tokio::test]
async fn a_denied_console_flow_reports_the_failure_and_is_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let env = provisioning(db).await;
        let start = env.oidc.begin_console(PROVIDER).await.unwrap();
        let request = AuthRequest::parse(&start.url);
        let denied = CallbackParams {
            state: Some(&request.state),
            code: None,
            error: Some("access_denied"),
        };
        let b = db.backend;

        let report = env
            .oidc
            .complete_console(&denied, Some(start.binding.expose()))
            .await
            .unwrap();

        assert_eq!(report.outcome.err(), Some(Failure::Denied), "{b}");
        assert_eq!(flows(db).await, 0, "{b}");
    })
    .await;
}
