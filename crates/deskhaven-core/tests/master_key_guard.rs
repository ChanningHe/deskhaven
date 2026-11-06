//! The master key at startup, against the migrated database: generated only into a
//! database without accounts; with accounts and no key, startup refuses (a new key would
//! orphan every sealed secret, and startup sealing would re-seal under it). Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use deskhaven_config::Secret;
use deskhaven_core::crypto::MasterKey;
use deskhaven_core::keys::{MasterKeyError, load_or_generate_master_key_for, master_key_path};
use deskhaven_core::users::{
    Credential, HashParams, NewUser, Password, PasswordHasher, UserService,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};

const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

async fn add_account(db: &TestDb) {
    let settings = common::settings(db).await;
    let users = UserService::new(
        db.conn.clone(),
        settings,
        PasswordHasher::new(FAST).unwrap(),
    );
    let credential = Credential::Local {
        password: Password::new("tundra-Lamp-42"),
    };
    users
        .create(NewUser::new("alice", credential))
        .await
        .unwrap();
}

fn data_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

#[tokio::test]
async fn an_empty_database_gets_a_generated_key() {
    each_migrated_backend(async |db: &TestDb| {
        let dir = data_dir();

        let loaded = load_or_generate_master_key_for(&Secret::default(), dir.path(), &db.conn)
            .await
            .unwrap();
        let again = load_or_generate_master_key_for(&Secret::default(), dir.path(), &db.conn)
            .await
            .unwrap();

        let b = db.backend;
        assert!(master_key_path(dir.path()).exists(), "{b}");
        assert_eq!(loaded.value.key_id(), again.value.key_id(), "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_database_with_accounts_and_no_key_refuses_and_writes_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        add_account(db).await;
        let dir = data_dir();

        let refused =
            load_or_generate_master_key_for(&Secret::default(), dir.path(), &db.conn).await;

        let b = db.backend;
        assert!(
            matches!(refused, Err(MasterKeyError::Missing { .. })),
            "{b}: {refused:?}"
        );
        assert!(
            !master_key_path(dir.path()).exists(),
            "{b}: no key generated"
        );
    })
    .await;
}

#[tokio::test]
async fn a_database_with_accounts_starts_with_its_key() {
    each_migrated_backend(async |db: &TestDb| {
        let dir = data_dir();
        let generated = load_or_generate_master_key_for(&Secret::default(), dir.path(), &db.conn)
            .await
            .unwrap();
        add_account(db).await;
        let key = MasterKey::from_bytes(&[5; 32]);
        let configured = Secret::new(key.to_base64().as_str());
        let elsewhere = data_dir();

        let from_file = load_or_generate_master_key_for(&Secret::default(), dir.path(), &db.conn)
            .await
            .unwrap();
        let from_config = load_or_generate_master_key_for(&configured, elsewhere.path(), &db.conn)
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(from_file.value.key_id(), generated.value.key_id(), "{b}");
        assert_eq!(from_config.value.key_id(), key.key_id(), "{b}");
    })
    .await;
}
