//! `deskhaven serve` with a lost master key: a database that already holds accounts makes
//! startup refuse (and generate nothing) instead of starting with a new key.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]

mod support;

use std::path::Path;

use deskhaven_db::entity::user::{self, AuthSource, Role, Status};
use deskhaven_db::{DbOptions, connect, time};
use sea_orm::ActiveModelTrait;
use sea_orm::ActiveValue::Set;
use support::*;

/// An account row, as the console would have created it.
async fn add_account(data: &Path) {
    let url = format!("sqlite://{}", data.join("deskhaven.db").display());
    let db = connect(&DbOptions::new(url)).await.unwrap();
    let now = time::now();
    user::ActiveModel {
        username: Set("alice".to_owned()),
        username_norm: Set("alice".to_owned()),
        display_name: Set("alice".to_owned()),
        email_verified: Set(false),
        auth_source: Set(AuthSource::Local),
        role: Set(Role::Owner),
        status: Set(Status::Active),
        must_change_password: Set(false),
        note: Set(String::new()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&db)
    .await
    .unwrap();
}

#[tokio::test]
async fn a_lost_master_key_with_accounts_refuses_to_start_and_generates_none() {
    let dir = workdir("master-key-lost");
    let data = dir.join("data");
    let mut first = start(&dir, &config(&dir, ""), &[]);
    first.listening("client_api");
    let (status, _) = first.terminate();
    assert_eq!(status.code(), Some(0), "{}", first.logs());
    add_account(&data).await;
    std::fs::remove_file(data.join("master.key")).unwrap();

    let mut second = start(&dir, &config(&dir, ""), &[]);
    let (status, _) = second.wait_exit(FAILURE_LIMIT);

    let logs = second.logs();
    assert!(!status.success(), "{logs}");
    assert!(logs.contains("no master key"), "{logs}");
    assert!(!data.join("master.key").exists(), "a new key was generated");
    assert!(!logs.contains("\"ready\""), "{logs}");
}
