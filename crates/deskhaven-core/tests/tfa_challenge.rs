//! TfaService login challenges with a manual clock: the ±1 step window, persisted replay
//! protection, single use, attempt cap, expiry, username/device binding, recovery codes
//! and cleanup. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::tfa::{Env, RUSTDESK_ID, device, first_code, pending};
use deskhaven_core::sessions::ClientDevice;
use deskhaven_core::tfa::{
    CHALLENGE_TTL, ChallengeError, MAX_CHALLENGE_ATTEMPTS, Passed, SecondFactorMethod,
};
use deskhaven_db::entity::{auth_challenge, user};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::{EntityTrait, PaginatorTrait};
use sha2::{Digest, Sha256};

const STEP: Duration = Duration::from_secs(30);

fn invalid(result: &Result<Passed, ChallengeError>) -> bool {
    matches!(result, Err(ChallengeError::Invalid))
}

fn wrong(result: &Result<Passed, ChallengeError>) -> bool {
    matches!(result, Err(ChallengeError::WrongCode))
}

#[tokio::test]
async fn current_code_passes_with_the_step_one_state() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP);
        let challenge = env.challenge(&alice).await;

        let passed = env
            .verify(&challenge, "Alice", &env.code(&alice))
            .await
            .unwrap();

        assert_eq!(passed.user.id, alice.user_id, "{}", db.backend);
        assert_eq!(passed.stamp, alice.stamp);
        assert_eq!(passed.lockout, pending("alice"));
        assert_eq!(passed.method, SecondFactorMethod::Totp);
        let row = user::Entity::find_by_id(alice.user_id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.totp_last_step,
            Some(deskhaven_core::tfa::totp::step_at(env.now_ms()))
        );
    })
    .await;
}

#[tokio::test]
async fn accepts_one_step_of_drift_either_way_and_no_more() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP * 10);
        let b = db.backend;

        let too_old = env
            .verify(
                &env.challenge(&alice).await,
                "alice",
                &env.code_in(&alice, -2),
            )
            .await;
        let too_new = env
            .verify(
                &env.challenge(&alice).await,
                "alice",
                &env.code_in(&alice, 2),
            )
            .await;
        let behind = env
            .verify(
                &env.challenge(&alice).await,
                "alice",
                &env.code_in(&alice, -1),
            )
            .await;
        let ahead = env
            .verify(
                &env.challenge(&alice).await,
                "alice",
                &env.code_in(&alice, 1),
            )
            .await;

        assert!(wrong(&too_old), "{b}: {too_old:?}");
        assert!(wrong(&too_new), "{b}: {too_new:?}");
        assert!(behind.is_ok(), "{b}: {behind:?}");
        assert!(ahead.is_ok(), "{b}: {ahead:?}");
    })
    .await;
}

#[tokio::test]
async fn a_code_works_once_and_older_steps_never_again() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP * 10);
        let current = env.code(&alice);
        let previous = env.code_in(&alice, -1);
        let b = db.backend;

        let first = env
            .verify(&env.challenge(&alice).await, "alice", &current)
            .await;
        let replay = env
            .verify(&env.challenge(&alice).await, "alice", &current)
            .await;
        let older = env
            .verify(&env.challenge(&alice).await, "alice", &previous)
            .await;
        env.clock.advance(STEP);
        let next = env
            .verify(&env.challenge(&alice).await, "alice", &env.code(&alice))
            .await;

        assert!(first.is_ok(), "{b}: {first:?}");
        assert!(wrong(&replay), "{b}: {replay:?}");
        assert!(wrong(&older), "{b}: {older:?}");
        assert!(next.is_ok(), "{b}: {next:?}");
    })
    .await;
}

#[tokio::test]
async fn the_enrolment_code_cannot_log_in() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;

        // Same step as the confirmation: already spent.
        let result = env
            .verify(&env.challenge(&alice).await, "alice", &env.code(&alice))
            .await;

        assert!(wrong(&result), "{}: {result:?}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn a_challenge_is_single_use() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP);
        let challenge = env.challenge(&alice).await;
        env.verify(&challenge, "alice", &env.code(&alice))
            .await
            .unwrap();
        env.clock.advance(STEP);

        let again = env.verify(&challenge, "alice", &env.code(&alice)).await;

        assert!(invalid(&again), "{}: {again:?}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn attempts_are_capped_even_for_the_right_code() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP);
        let challenge = env.challenge(&alice).await;

        for _ in 0..MAX_CHALLENGE_ATTEMPTS {
            let result = env.verify(&challenge, "alice", "000000").await;
            assert!(wrong(&result), "{}: {result:?}", db.backend);
        }
        let right = env.verify(&challenge, "alice", &env.code(&alice)).await;

        assert!(invalid(&right), "{}: {right:?}", db.backend);
        let row = auth_challenge::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.attempts, MAX_CHALLENGE_ATTEMPTS);
        assert_eq!(row.consumed_at, None);
    })
    .await;
}

#[tokio::test]
async fn an_expired_or_unknown_challenge_is_invalid() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        let challenge = env.challenge(&alice).await;
        env.clock.advance(CHALLENGE_TTL);
        let b = db.backend;

        let expired = env.verify(&challenge, "alice", &env.code(&alice)).await;
        let other = env.challenge(&alice).await;
        let mut tampered = other.expose().to_owned();
        tampered.replace_range(..1, if tampered.starts_with('A') { "B" } else { "A" });
        let unknown = env
            .tfa
            .verify_challenge(&deskhaven_core::tfa::SecondStep {
                secret: &tampered,
                username: "alice",
                device: &device(),
                code: &env.code(&alice),
            })
            .await;
        let malformed = env
            .tfa
            .verify_challenge(&deskhaven_core::tfa::SecondStep {
                secret: "not a secret",
                username: "alice",
                device: &device(),
                code: &env.code(&alice),
            })
            .await;

        assert!(invalid(&expired), "{b}: {expired:?}");
        assert!(invalid(&unknown), "{b}: {unknown:?}");
        assert!(invalid(&malformed), "{b}: {malformed:?}");
    })
    .await;
}

#[tokio::test]
async fn a_challenge_is_bound_to_its_username_and_device() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.enrolled("bob").await;
        env.clock.advance(STEP);
        let challenge = env.challenge(&alice).await;
        let other_id = ClientDevice {
            rustdesk_id: "987654321".into(),
            ..device()
        };
        let other_uuid = ClientDevice {
            uuid: "b3RoZXI=".into(),
            ..device()
        };
        let code = env.code(&alice);
        let b = db.backend;

        let as_bob = env.verify(&challenge, "bob", &code).await;
        let from_id = env.verify_from(&challenge, "alice", &other_id, &code).await;
        let from_uuid = env
            .verify_from(&challenge, "alice", &other_uuid, &code)
            .await;
        let right = env.verify(&challenge, "ALICE", &code).await;

        assert!(invalid(&as_bob), "{b}: {as_bob:?}");
        assert!(invalid(&from_id), "{b}: {from_id:?}");
        assert!(invalid(&from_uuid), "{b}: {from_uuid:?}");
        assert!(right.is_ok(), "{b}: {right:?}");
        assert_eq!(other_id.uuid, device().uuid);
        assert_eq!(other_uuid.rustdesk_id, RUSTDESK_ID);
    })
    .await;
}

#[tokio::test]
async fn a_recovery_code_works_once_in_any_spelling() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        let code = first_code(&alice.recovery);
        let spelled = code.replace('-', " ").to_uppercase();
        let b = db.backend;

        let first = env
            .verify(&env.challenge(&alice).await, "alice", &spelled)
            .await;
        let again = env
            .verify(&env.challenge(&alice).await, "alice", &code)
            .await;
        let garbage = env
            .verify(&env.challenge(&alice).await, "alice", "abcde-fghij")
            .await;

        assert_eq!(first.unwrap().method, SecondFactorMethod::Recovery, "{b}");
        assert!(wrong(&again), "{b}: {again:?}");
        assert!(wrong(&garbage), "{b}: {garbage:?}");
        assert_eq!(
            env.tfa
                .status(alice.user_id)
                .await
                .unwrap()
                .recovery_codes_left,
            9
        );
    })
    .await;
}

#[tokio::test]
async fn only_the_secret_hash_is_stored_and_cleanup_drops_finished_challenges() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP);
        let used = env.challenge(&alice).await;
        env.verify(&used, "alice", &env.code(&alice)).await.unwrap();
        let live = env.challenge(&alice).await;

        let rows = auth_challenge::Entity::find().all(&db.conn).await.unwrap();
        let hash = format!("{:x}", Sha256::digest(live.expose().as_bytes()));
        assert!(rows.iter().any(|r| r.secret_hash == hash));
        assert!(rows.iter().all(|r| !r.context.contains(live.expose())));
        assert!(
            rows.iter()
                .all(|r| r.kind == auth_challenge::Kind::Totp && r.code_hash.is_none())
        );

        assert_eq!(
            env.tfa.cleanup_challenges().await.unwrap(),
            1,
            "{}",
            db.backend
        );
        env.clock.advance(CHALLENGE_TTL);
        assert_eq!(env.tfa.cleanup_challenges().await.unwrap(), 1);
        assert_eq!(
            auth_challenge::Entity::find()
                .count(&db.conn)
                .await
                .unwrap(),
            0
        );
    })
    .await;
}
