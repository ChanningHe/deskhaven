//! End-to-end tests of `deskhaven user unlock` against a migrated SQLite database.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use deskhaven_db::entity::console_audit::ActorKind;
use deskhaven_db::entity::{auth_lockout, console_audit};
use deskhaven_db::{DbOptions, connect, migrate, time};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait};

fn workdir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("cli_user")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(
        dir.join("none.toml"),
        "[listen]\nclient_api = \"127.0.0.1:0\"\n\n[signal]\nenabled = false\n",
    )
    .unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    let data = dir.join("data");
    Command::new(env!("CARGO_BIN_EXE_deskhaven"))
        .args([
            "--config",
            "none.toml",
            "--data-dir",
            data.to_str().unwrap(),
        ])
        .args(args)
        .env_clear()
        .current_dir(dir)
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

async fn database(dir: &Path) -> DatabaseConnection {
    let url = format!("sqlite://{}", dir.join("data/deskhaven.db").display());
    let db = connect(&DbOptions::new(url)).await.unwrap();
    migrate(&db).await.unwrap();
    db
}

async fn lockout_row(db: &DatabaseConnection, key: &str) {
    let now = time::now();
    auth_lockout::ActiveModel {
        key: Set(key.to_owned()),
        failures: Set(5),
        window_started_at: Set(now),
        locked_until: Set(Some(now)),
        lock_level: Set(2),
        updated_at: Set(now),
        last_success_at: Set(None),
    }
    .insert(db)
    .await
    .unwrap();
}

#[tokio::test]
async fn unlock_clears_only_that_usernames_rows_and_is_audited() {
    let dir = workdir("clears");
    let db = database(&dir).await;
    for key in ["user:bob|*", "user:bob|192.0.2.1", "user:bobby|*"] {
        lockout_row(&db, key).await;
    }

    let out = run(&dir, &["user", "unlock", "BOB"]);

    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "unlocked BOB: 2 lockout entries cleared\n"
    );
    let keys: Vec<String> = auth_lockout::Entity::find()
        .all(&db)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.key)
        .collect();
    assert_eq!(keys, ["user:bobby|*"]);
    let audit = console_audit::Entity::find().all(&db).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].actor_kind, ActorKind::Cli);
    assert_eq!(
        (audit[0].action.as_str(), audit[0].target_id.as_str()),
        ("user.unlock", "BOB")
    );
    assert!(
        audit[0]
            .after
            .as_deref()
            .unwrap()
            .contains("\"lockout_entries_cleared\":2")
    );

    let again = run(&dir, &["user", "unlock", "bob"]);
    assert_eq!(
        text(&again.stdout),
        "unlocked bob: 0 lockout entries cleared\n"
    );
}

#[tokio::test]
async fn unlock_refuses_bad_usernames_and_missing_databases() {
    let dir = workdir("refuses");

    let missing = run(&dir, &["user", "unlock", "bob"]);
    assert!(!missing.status.success());
    assert!(
        text(&missing.stderr).contains("does not exist yet"),
        "{}",
        text(&missing.stderr)
    );
    assert!(
        !dir.join("data/deskhaven.db").exists(),
        "must not create the database"
    );

    let bad = run(&dir, &["user", "unlock", "bad name"]);
    assert!(!bad.status.success());
    assert!(text(&bad.stderr).contains("not a valid username"));
}
