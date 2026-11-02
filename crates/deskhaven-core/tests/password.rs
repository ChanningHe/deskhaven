//! Argon2id hashing, rehash detection, dummy verification and the password policy (06 §3.1).

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::{Duration, Instant};

use deskhaven_core::users::{
    HashLimits, HashParams, MAX_PASSWORD_LEN, Password, PasswordError, PasswordHasher,
    PolicyViolation, check_password_policy,
};

const DEFAULT_PREFIX: &str = "$argon2id$v=19$m=19456,t=2,p=1$";
const CHEAP: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};
/// Production parameters take tens of milliseconds even on fast hardware.
const MIN_HASH_TIME: Duration = Duration::from_millis(5);

fn pw(s: &str) -> Password {
    Password::new(s)
}

#[tokio::test]
async fn available_slots_shows_a_running_computation_and_is_shared_by_clones() {
    let limits = HashLimits {
        max_concurrent: 2,
        acquire_timeout: Duration::from_secs(5),
    };
    // Slow enough (hundreds of ms) that the loop below sees the computation running.
    let slow = HashParams {
        iterations: 16,
        ..HashParams::DEFAULT
    };
    let hasher = PasswordHasher::with_limits(slow, limits).unwrap();
    let clone = hasher.clone();
    assert_eq!(hasher.available_slots(), 2);

    let running = tokio::spawn(async move { clone.verify_dummy(&pw("x")).await });
    while hasher.available_slots() == 2 {
        tokio::task::yield_now().await;
    }
    assert_eq!(hasher.available_slots(), 1);
    running.await.unwrap().unwrap();

    assert_eq!(hasher.available_slots(), 2);
}

#[tokio::test]
async fn hash_is_argon2id_phc_with_documented_params() {
    let hasher = PasswordHasher::default();

    let phc = hasher.hash(&pw("correct horse battery")).await.unwrap();

    assert!(phc.starts_with(DEFAULT_PREFIX), "{phc}");
    assert!(
        hasher
            .verify(&pw("correct horse battery"), &phc)
            .await
            .unwrap()
    );
    assert!(
        !hasher
            .verify(&pw("Correct horse battery"), &phc)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn each_hash_has_its_own_salt() {
    let hasher = PasswordHasher::new(CHEAP).unwrap();

    let a = hasher.hash(&pw("same password")).await.unwrap();
    let b = hasher.hash(&pw("same password")).await.unwrap();

    assert_ne!(a, b);
}

#[tokio::test]
async fn unicode_passwords_round_trip() {
    let hasher = PasswordHasher::new(CHEAP).unwrap();
    let secret = "pässwörd-密码-🔑-Ωmega";

    let phc = hasher.hash(&pw(secret)).await.unwrap();

    assert!(hasher.verify(&pw(secret), &phc).await.unwrap());
    assert!(
        !hasher
            .verify(&pw("passwörd-密码-🔑-Ωmega"), &phc)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn hash_with_other_params_verifies_but_needs_rehash() {
    let old = PasswordHasher::new(CHEAP).unwrap();
    let current = PasswordHasher::default();
    let phc = old.hash(&pw("rotate me please")).await.unwrap();

    assert!(current.verify(&pw("rotate me please"), &phc).await.unwrap());
    assert!(current.needs_rehash(&phc));
    assert!(!old.needs_rehash(&phc));

    let fresh = current.hash(&pw("rotate me please")).await.unwrap();
    assert!(!current.needs_rehash(&fresh));
}

#[tokio::test]
async fn foreign_or_malformed_hashes_need_rehash() {
    let hasher = PasswordHasher::new(CHEAP).unwrap();
    let argon2i = "$argon2i$v=19$m=1024,t=1,p=1$c29tZXNhbHRzb21lc2FsdA$\
                   9sTbSlTio3Biev89thdrlKKiCaYsjjYVJxGAL3swxpQ";

    assert!(hasher.needs_rehash("not a hash"));
    assert!(hasher.needs_rehash(argon2i));
    assert!(hasher.needs_rehash("$2b$12$abcdefghijklmnopqrstuuJ0j7k1eWm8N0rD2C3o5QjVzW8uY0GQ6"));
}

#[tokio::test]
async fn malformed_stored_hash_is_an_error_not_a_mismatch() {
    let hasher = PasswordHasher::new(CHEAP).unwrap();

    let result = hasher.verify(&pw("whatever"), "$argon2id$garbage").await;

    assert!(matches!(result, Err(PasswordError::MalformedHash)));
}

#[tokio::test]
async fn invalid_params_are_rejected() {
    let zero_time = HashParams {
        iterations: 0,
        ..CHEAP
    };

    assert!(matches!(
        PasswordHasher::new(zero_time),
        Err(PasswordError::Params)
    ));
}

#[tokio::test]
async fn dummy_verification_costs_about_as_much_as_a_real_one() {
    let hasher = PasswordHasher::default();
    let phc = hasher.hash(&pw("the real password")).await.unwrap();

    let start = Instant::now();
    assert!(!hasher.verify(&pw("a wrong guess"), &phc).await.unwrap());
    let real = start.elapsed();

    let start = Instant::now();
    hasher.verify_dummy(&pw("a wrong guess")).await.unwrap();
    let dummy = start.elapsed();

    assert!(real > MIN_HASH_TIME, "real verify took {real:?}");
    assert!(dummy > MIN_HASH_TIME, "dummy verify took {dummy:?}");
    // Loose bound: same work, but CI machines are noisy.
    assert!(
        dummy * 4 > real && real * 4 > dummy,
        "{real:?} vs {dummy:?}"
    );
}

#[test]
fn password_debug_never_shows_the_value() {
    let secret = pw("hunter2-but-longer");

    let rendered = format!("{secret:?}");

    assert!(!rendered.contains("hunter2"), "{rendered}");
}

#[test]
fn policy_length_bounds_count_characters() {
    let min = 10;

    assert_eq!(
        check_password_policy(&pw("Abcdefgh9"), min),
        Err(PolicyViolation::TooShort { min })
    );
    assert_eq!(check_password_policy(&pw("Abcdefgh9!"), min), Ok(()));
    // 10 characters, 30 bytes.
    assert_eq!(
        check_password_policy(&pw("密码密码密码密码密码"), min),
        Ok(())
    );
    assert_eq!(
        check_password_policy(&pw("密码密码密码密码密"), min),
        Err(PolicyViolation::TooShort { min })
    );

    let longest = "x7".repeat(MAX_PASSWORD_LEN / 2);
    assert_eq!(check_password_policy(&pw(&longest), min), Ok(()));
    let too_long = format!("{longest}y");
    assert_eq!(
        check_password_policy(&pw(&too_long), min),
        Err(PolicyViolation::TooLong {
            max: MAX_PASSWORD_LEN
        })
    );
}

#[test]
fn policy_rejects_common_passwords_case_insensitively() {
    for common in [
        "password1",
        "PASSWORD1",
        "PassWord1",
        "qwertyuiop",
        "12345678",
    ] {
        assert_eq!(
            check_password_policy(&pw(common), 8),
            Err(PolicyViolation::Common),
            "{common}"
        );
    }
    assert_eq!(check_password_policy(&pw("vXq9-tundra-Lamp"), 8), Ok(()));
}

#[test]
fn policy_allows_spaces_and_emoji() {
    assert_eq!(
        check_password_policy(&pw("🔑🔑 my long passphrase"), 10),
        Ok(())
    );
}
