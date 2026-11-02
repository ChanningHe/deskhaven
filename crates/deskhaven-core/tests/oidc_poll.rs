//! Client polling of an OIDC flow: pending, single hand-out, device binding, failures,
//! expiry, and the session minted from the stored credential stamp. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::oidc::{Env, PASSWORD, RUSTDESK_ID, UUID, device};
use deskhaven_core::oidc::{ClientLogin, FLOW_TTL, Failure, PollError, PollOutcome};
use deskhaven_core::sessions::{SessionError, SessionKind};
use deskhaven_core::users::{Password, Status};
use deskhaven_db::entity::oidc_flow;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};

fn ready(outcome: Result<PollOutcome, PollError>) -> ClientLogin {
    match outcome {
        Ok(PollOutcome::Ready(login)) => *login,
        other => panic!("not ready: {other:?}"),
    }
}

#[tokio::test]
async fn pending_until_the_callback_then_ready_exactly_once() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (start, request) = env.begin().await;
        let code = start.code.expose();

        let before = env.oidc.poll_client(code, RUSTDESK_ID, UUID).await;
        env.idp.mount_token(&env.spec("sub-1", &request)).await;
        let report = env.callback(&request).await;
        let first = ready(env.oidc.poll_client(code, RUSTDESK_ID, UUID).await);
        let second = env.oidc.poll_client(code, RUSTDESK_ID, UUID).await;

        assert!(
            matches!(before, Ok(PollOutcome::Pending)),
            "{}: {before:?}",
            db.backend
        );
        assert_eq!(first.login.user.id, report.outcome.unwrap().user.id);
        assert_eq!(first.device.rustdesk_id, RUSTDESK_ID);
        assert_eq!(first.device.uuid, UUID);
        assert_eq!(first.device.info, device().info);
        assert!(matches!(second, Err(PollError::Invalid)), "{second:?}");
        assert_eq!(oidc_flow::Entity::find().count(&db.conn).await.unwrap(), 0);
    })
    .await;
}

#[tokio::test]
async fn another_device_cannot_poll_the_flow_and_does_not_consume_it() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (start, request) = env.begin().await;
        env.idp.mount_token(&env.spec("sub-1", &request)).await;
        env.callback(&request).await;
        let code = start.code.expose();

        let other_id = env.oidc.poll_client(code, "987654321", UUID).await;
        let other_uuid = env.oidc.poll_client(code, RUSTDESK_ID, "b3RoZXI=").await;
        let own = env.oidc.poll_client(code, RUSTDESK_ID, UUID).await;

        assert!(
            matches!(other_id, Err(PollError::Invalid)),
            "{}",
            db.backend
        );
        assert!(matches!(other_uuid, Err(PollError::Invalid)));
        ready(own);
    })
    .await;
}

#[tokio::test]
async fn a_failed_flow_reports_its_failure_on_every_poll() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (start, _) = env.login("unknown-sub", |s| s).await;
        let code = start.code.expose();

        for _ in 0..2 {
            let outcome = env.oidc.poll_client(code, RUSTDESK_ID, UUID).await;
            assert!(
                matches!(outcome, Ok(PollOutcome::Failed(Failure::NoLinkedAccount))),
                "{}: {outcome:?}",
                db.backend
            );
        }
    })
    .await;
}

#[tokio::test]
async fn unknown_malformed_and_expired_codes_are_invalid() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (start, request) = env.begin().await;
        env.idp.mount_token(&env.spec("sub-1", &request)).await;
        env.callback(&request).await;
        let unknown = "B".repeat(43);

        let malformed = env.oidc.poll_client("not-a-code", RUSTDESK_ID, UUID).await;
        let missing = env.oidc.poll_client(&unknown, RUSTDESK_ID, UUID).await;
        env.clock.advance(FLOW_TTL);
        let expired = env
            .oidc
            .poll_client(start.code.expose(), RUSTDESK_ID, UUID)
            .await;

        for outcome in [malformed, missing, expired] {
            assert!(
                matches!(outcome, Err(PollError::Invalid)),
                "{}: {outcome:?}",
                db.backend
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_session_is_minted_for_the_flow_device_under_the_callback_stamp() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (start, _) = env.login("sub-1", |s| s).await;
        let login = ready(
            env.oidc
                .poll_client(start.code.expose(), RUSTDESK_ID, UUID)
                .await,
        );

        let (session, token) = env
            .sessions
            .create_client(login.login.user.id, &login.login.stamp, &login.device, None)
            .await
            .unwrap();
        let principal = env
            .sessions
            .authenticate(token.expose(), SessionKind::Client)
            .await
            .unwrap();

        assert_eq!(principal.user_id, login.login.user.id, "{}", db.backend);
        let bound = session.device.unwrap();
        assert_eq!(
            (bound.rustdesk_id.as_str(), bound.uuid.as_str()),
            (RUSTDESK_ID, UUID)
        );
    })
    .await;
}

#[tokio::test]
async fn a_password_change_or_disable_after_the_callback_refuses_the_session() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.local_user("alice", Some("alice@example.com")).await;
        let bob = env.local_user("bob", Some("bob@example.com")).await;
        let (alice_flow, _) = env
            .login("sub-a", |s| s.email("alice@example.com", Some(true)))
            .await;
        let (bob_flow, _) = env
            .login("sub-b", |s| s.email("bob@example.com", Some(true)))
            .await;
        let alice_login = ready(
            env.oidc
                .poll_client(alice_flow.code.expose(), RUSTDESK_ID, UUID)
                .await,
        );
        let bob_login = ready(
            env.oidc
                .poll_client(bob_flow.code.expose(), RUSTDESK_ID, UUID)
                .await,
        );

        let new_password = Password::new(format!("{PASSWORD}-new"));
        env.users
            .set_password(alice.id, &new_password, false)
            .await
            .unwrap();
        env.users
            .set_status(bob.id, Status::Disabled)
            .await
            .unwrap();
        let alice_session = env
            .sessions
            .create_client(
                alice.id,
                &alice_login.login.stamp,
                &alice_login.device,
                None,
            )
            .await;
        let bob_session = env
            .sessions
            .create_client(bob.id, &bob_login.login.stamp, &bob_login.device, None)
            .await;

        assert!(
            matches!(alice_session, Err(SessionError::StaleCredential)),
            "{}",
            db.backend
        );
        assert!(matches!(bob_session, Err(SessionError::StaleCredential)));
    })
    .await;
}
