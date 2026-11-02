//! TfaService enrolment and management: begin -> confirm -> enabled with recovery codes,
//! disable, regeneration, the sealed seed and `auth.tfa.enforce`. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::set;
use common::tfa::{Env, first_code};
use deskhaven_core::tfa::totp::{self, TotpSecret};
use deskhaven_core::tfa::{ChallengeError, RECOVERY_CODE_COUNT, SecondFactor, TfaError, TfaStatus};
use deskhaven_core::users::{Role, User};
use deskhaven_db::entity::{auth_challenge, recovery_code, user};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde_json::json;

async fn user_row(db: &TestDb, id: i64) -> user::Model {
    user::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn begin_then_confirm_enables_totp_and_issues_recovery_codes() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, _) = env.user("alice", Role::User).await;

        let enrolment = env.tfa.begin_enrolment(id).await.unwrap();
        let pending = env.tfa.status(id).await.unwrap();
        let secret = TotpSecret::from_base32(&enrolment.secret).unwrap();
        let wrong = env
            .tfa
            .confirm_enrolment(id, &totp::code_at(&secret, env.now_ms() + 5 * 60_000))
            .await;
        let codes = env
            .tfa
            .confirm_enrolment(id, &totp::code_at(&secret, env.now_ms()))
            .await
            .unwrap();

        let b = db.backend;
        assert!(
            enrolment
                .uri
                .starts_with("otpauth://totp/deskhaven:alice?secret="),
            "{b}"
        );
        assert!(enrolment.uri.contains(enrolment.secret.as_str()));
        assert_eq!(
            pending,
            TfaStatus {
                enabled: false,
                pending_enrolment: true,
                recovery_codes_left: 0
            }
        );
        assert!(
            matches!(wrong, Err(TfaError::InvalidCode)),
            "{b}: {wrong:?}"
        );
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);
        assert_eq!(
            env.tfa.status(id).await.unwrap(),
            TfaStatus {
                enabled: true,
                pending_enrolment: false,
                recovery_codes_left: 10
            }
        );
        let row = user_row(db, id).await;
        assert!(row.totp_enabled_at.is_some());
        // The confirming code's step is spent: it cannot also log in.
        assert_eq!(row.totp_last_step, Some(totp::step_at(env.now_ms())));
        assert!(env.users.get(id).await.unwrap().totp_enabled);
    })
    .await;
}

#[tokio::test]
async fn seed_is_sealed_for_its_row_and_recovery_codes_are_hashed() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        let bob = env.enrolled("bob").await;

        let row = user_row(db, alice.user_id).await;
        let sealed = row.totp_secret_enc.unwrap();
        let base32 = alice.secret.to_base32();
        assert!(sealed.starts_with("v1:"));
        assert!(!sealed.contains(base32.as_str()));
        let ctx = format!("users.totp_secret_enc:{}", alice.user_id);
        let other = format!("users.totp_secret_enc:{}", bob.user_id);
        assert!(env.crypto.open(&ctx, &sealed).is_ok());
        assert!(env.crypto.open(&other, &sealed).is_err());

        let hashes: Vec<String> = recovery_code::Entity::find()
            .filter(recovery_code::Column::UserId.eq(alice.user_id))
            .all(&db.conn)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.code_hash)
            .collect();
        assert_eq!(hashes.len(), RECOVERY_CODE_COUNT);
        for code in alice.recovery.codes() {
            let plain = code.replace('-', "");
            assert!(
                hashes
                    .iter()
                    .all(|h| !h.contains(&plain) && h.starts_with("v1:"))
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_seed_moved_to_another_row_does_not_verify() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        let mallory = env.enrolled("mallory").await;
        let alice_sealed = user_row(db, alice.user_id).await.totp_secret_enc.unwrap();
        user::Entity::update_many()
            .col_expr(user::Column::TotpSecretEnc, Expr::value(alice_sealed))
            .filter(user::Column::Id.eq(mallory.user_id))
            .exec(&db.conn)
            .await
            .unwrap();
        env.clock.advance(common::MINUTE);
        let challenge = env.challenge(&mallory).await;

        // Mallory knows Alice's seed (her own authenticator would not match either).
        let result = env.verify(&challenge, "mallory", &env.code(&alice)).await;

        assert!(
            matches!(result, Err(ChallengeError::Internal(TfaError::Corrupt))),
            "{}: {result:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn enrolment_state_errors() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (plain, _) = env.user("plain", Role::User).await;
        let enrolled = env.enrolled("alice").await;
        let b = db.backend;

        let no_pending = env.tfa.confirm_enrolment(plain, "123456").await;
        let again = env.tfa.begin_enrolment(enrolled.user_id).await;
        let confirm_enabled = env.tfa.confirm_enrolment(enrolled.user_id, "123456").await;
        let regen_plain = env.tfa.regenerate_recovery_codes(plain).await;
        let missing = env.tfa.begin_enrolment(9_999).await;
        let missing_status = env.tfa.status(9_999).await;

        assert!(
            matches!(no_pending, Err(TfaError::NoPendingEnrolment)),
            "{b}: {no_pending:?}"
        );
        assert!(
            matches!(again, Err(TfaError::AlreadyEnabled)),
            "{b}: {again:?}"
        );
        assert!(
            matches!(confirm_enabled, Err(TfaError::AlreadyEnabled)),
            "{b}"
        );
        assert!(
            matches!(regen_plain, Err(TfaError::NotEnabled)),
            "{b}: {regen_plain:?}"
        );
        assert!(
            matches!(missing, Err(TfaError::NotFound)),
            "{b}: {missing:?}"
        );
        assert!(matches!(missing_status, Err(TfaError::NotFound)), "{b}");
    })
    .await;
}

#[tokio::test]
async fn regenerating_replaces_every_recovery_code() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        let old = first_code(&alice.recovery);

        let fresh = env
            .tfa
            .regenerate_recovery_codes(alice.user_id)
            .await
            .unwrap();
        let old_result = env
            .verify(&env.challenge(&alice).await, "alice", &old)
            .await;
        let new_result = env
            .verify(&env.challenge(&alice).await, "alice", &first_code(&fresh))
            .await;

        let b = db.backend;
        assert!(
            matches!(old_result, Err(ChallengeError::WrongCode)),
            "{b}: {old_result:?}"
        );
        assert!(new_result.is_ok(), "{b}: {new_result:?}");
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
async fn disable_forgets_seed_codes_and_pending_challenges() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let alice = env.enrolled("alice").await;
        let challenge = env.challenge(&alice).await;
        env.clock.advance(common::MINUTE);

        env.tfa.disable(alice.user_id).await.unwrap();
        env.tfa.disable(alice.user_id).await.unwrap();

        let row = user_row(db, alice.user_id).await;
        assert_eq!(
            (row.totp_secret_enc, row.totp_enabled_at, row.totp_last_step),
            (None, None, None)
        );
        let codes = recovery_code::Entity::find().count(&db.conn).await.unwrap();
        let challenges = auth_challenge::Entity::find()
            .count(&db.conn)
            .await
            .unwrap();
        assert_eq!((codes, challenges), (0, 0), "{}", db.backend);
        let result = env.verify(&challenge, "alice", &env.code(&alice)).await;
        assert!(matches!(result, Err(ChallengeError::Invalid)), "{result:?}");
        // A fresh enrolment can start.
        assert!(env.tfa.begin_enrolment(alice.user_id).await.is_ok());
    })
    .await;
}

#[tokio::test]
async fn enforce_setting_decides_who_must_enrol() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (user_id, _) = env.user("user", Role::User).await;
        let (admin_id, _) = env.user("admin", Role::Admin).await;
        let enrolled = env.enrolled("alice").await;
        let user = env.users.get(user_id).await.unwrap();
        let admin = env.users.get(admin_id).await.unwrap();
        let alice = env.users.get(enrolled.user_id).await.unwrap();
        // The role is read from the user as given: no need for a second owner account.
        let owner = User {
            role: Role::Owner,
            ..user.clone()
        };

        let mut seen = Vec::new();
        for mode in ["off", "admins", "all"] {
            set(&env.settings, "auth.tfa.enforce", json!(mode)).await;
            seen.push([&user, &admin, &owner, &alice].map(|u| env.tfa.requirement(u)));
        }

        use SecondFactor::{NotRequired as No, Required as Yes, SetupRequired as Setup};
        assert_eq!(
            seen,
            [
                [No, No, No, Yes],
                [No, Setup, Setup, Yes],
                [Setup, Setup, Setup, Yes]
            ],
            "{}",
            db.backend
        );
    })
    .await;
}
