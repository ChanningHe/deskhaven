//! TfaService under concurrent second steps: the challenge attempt cap, the TOTP replay
//! guard and single use hold when requests race. Both backends (PostgreSQL included: row
//! locks and conditional UPDATEs, not SQLite's single writer, are what is tested there).

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::tfa::{Enrolled, Env, device, first_code};
use deskhaven_core::tfa::{
    ChallengeError, LoginBinding, MAX_CHALLENGE_ATTEMPTS, Passed, SecondFactorMethod, SecondStep,
    TfaService, totp,
};
use deskhaven_db::entity::{auth_challenge, user};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;
use tokio::task::JoinSet;

const STEP: Duration = Duration::from_secs(30);
const RACERS: usize = 12;

type Outcome = Result<Passed, ChallengeError>;

/// Runs every `(secret, code)` second step of `alice` at once.
async fn race(tfa: &TfaService, steps: Vec<(String, String)>) -> Vec<Outcome> {
    let mut set = JoinSet::new();
    for (secret, code) in steps {
        let tfa = tfa.clone();
        set.spawn(async move {
            let device = device();
            let step = SecondStep {
                secret: &secret,
                username: "alice",
                binding: LoginBinding::Device(&device),
                code: &code,
            };
            tfa.verify_challenge(&step).await
        });
    }
    set.join_all().await
}

fn count(outcomes: &[Outcome], f: impl Fn(&Outcome) -> bool) -> usize {
    outcomes.iter().filter(|o| f(o)).count()
}

fn is_wrong(o: &Outcome) -> bool {
    matches!(o, Err(ChallengeError::WrongCode))
}

fn is_invalid(o: &Outcome) -> bool {
    matches!(o, Err(ChallengeError::Invalid))
}

async fn last_step(db: &TestDb, user: &Enrolled) -> Option<i64> {
    let row = user::Entity::find_by_id(user.user_id).one(&db.conn).await;
    row.unwrap().unwrap().totp_last_step
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_claims_never_exceed_the_attempt_cap() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP);
        let secret = env.challenge(&alice).await.expose().to_owned();
        let wrong = env.wrong_code(&alice);

        let steps = vec![(secret, wrong); RACERS];
        let outcomes = race(&env.tfa, steps).await;

        let b = db.backend;
        let cap = usize::try_from(MAX_CHALLENGE_ATTEMPTS).unwrap();
        assert_eq!(count(&outcomes, is_wrong), cap, "{b}: {outcomes:?}");
        assert_eq!(
            count(&outcomes, is_invalid),
            RACERS - cap,
            "{b}: {outcomes:?}"
        );
        let row = auth_challenge::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.attempts, MAX_CHALLENGE_ATTEMPTS, "{b}");
        assert_eq!(row.consumed_at, None, "{b}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_totp_code_on_two_challenges_at_once_logs_in_once() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        env.clock.advance(STEP);
        let code = env.code(&alice);
        let first = env.challenge(&alice).await.expose().to_owned();
        let second = env.challenge(&alice).await.expose().to_owned();

        let outcomes = race(&env.tfa, vec![(first, code.clone()), (second, code)]).await;

        let b = db.backend;
        assert_eq!(count(&outcomes, Result::is_ok), 1, "{b}: {outcomes:?}");
        assert_eq!(count(&outcomes, is_wrong), 1, "{b}: {outcomes:?}");
        assert_eq!(
            last_step(db, &alice).await,
            Some(totp::step_at(env.now_ms()))
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn totp_and_recovery_code_on_one_challenge_spend_only_the_winner() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        let enrolled_step = last_step(db, &alice).await;
        env.clock.advance(STEP);
        let secret = env.challenge(&alice).await.expose().to_owned();
        let steps = vec![
            (secret.clone(), env.code(&alice)),
            (secret, first_code(&alice.recovery)),
        ];

        let outcomes = race(&env.tfa, steps).await;

        let b = db.backend;
        assert_eq!(count(&outcomes, Result::is_ok), 1, "{b}: {outcomes:?}");
        assert_eq!(count(&outcomes, is_invalid), 1, "{b}: {outcomes:?}");
        let winner = outcomes
            .iter()
            .find_map(|o| o.as_ref().ok())
            .unwrap()
            .method;
        let left = env
            .tfa
            .status(alice.user_id)
            .await
            .unwrap()
            .recovery_codes_left;
        let step = last_step(db, &alice).await;
        match winner {
            SecondFactorMethod::Totp => {
                assert_eq!(left, 10, "{b}: the recovery code must stay unspent");
                assert_eq!(step, Some(totp::step_at(env.now_ms())), "{b}");
            }
            SecondFactorMethod::Recovery => {
                assert_eq!(left, 9, "{b}");
                assert_eq!(step, enrolled_step, "{b}: the TOTP step must stay unspent");
            }
        }
        let row = auth_challenge::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(row.consumed_at.is_some(), "{b}");
    })
    .await;
}
