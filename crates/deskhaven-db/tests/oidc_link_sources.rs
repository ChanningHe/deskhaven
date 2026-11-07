//! M4-09 migrations on existing data: `user_identities.linked_via` (the first identity of an
//! `oidc` account is `provisioned`, every other one `email`), `oidc_flows` admitting `link`
//! flows (SQLite rebuilds the table: rows and indexes survive), and
//! `auth.oidc.link_verified_email` becoming `auth.oidc.link_email_max_role`. Both backends.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ops::assert_checks;
use common::{T0_MS, exec, flow_row, provider_row, ts, user_row};
use deskhaven_db::begin_write;
use deskhaven_db::entity::oidc_flow::{self, Flow};
use deskhaven_db::entity::user::AuthSource;
use deskhaven_db::entity::user_identity::{self, LinkedVia};
use deskhaven_db::entity::{setting, setting_history};
use deskhaven_db::migration::Migrator;
use deskhaven_db::testing::{TestDb, each_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
};
use sea_orm_migration::MigratorTrait;

const FIRST: &str = "m20261107_000030_oidc_link_sources";

async fn migrate_steps(conn: &DatabaseConnection, steps: Option<u32>) {
    let txn = begin_write(conn).await.unwrap();
    Migrator::up(&txn, steps).await.unwrap();
    txn.commit().await.unwrap();
}

async fn migrate_before_m4_09(conn: &DatabaseConnection) {
    let before = Migrator::migrations()
        .iter()
        .position(|m| m.name() == FIRST)
        .unwrap();
    migrate_steps(conn, Some(u32::try_from(before).unwrap())).await;
}

async fn user(conn: &DatabaseConnection, name: &str, source: AuthSource) -> i64 {
    let mut row = user_row(name);
    if source != AuthSource::Local {
        row.password_hash = Set(None);
        row.password_changed_at = Set(None);
    }
    row.auth_source = Set(source);
    row.insert(conn).await.unwrap().id
}

/// An identity row as written before the column existed.
async fn old_identity(db: &TestDb, user_id: i64, provider_id: i64, subject: &str) {
    let sql = format!(
        "INSERT INTO user_identities (user_id, provider_id, subject, email_verified, created_at) \
         VALUES ({user_id}, {provider_id}, '{subject}', {}, {T0_MS})",
        if db.backend.to_string() == "sqlite" {
            "0"
        } else {
            "false"
        }
    );
    exec(&db.conn, &sql).await.unwrap();
}

async fn old_setting(conn: &DatabaseConnection, value: &str) {
    let sql = format!(
        "INSERT INTO settings (key, value, version, updated_by, updated_at) \
         VALUES ('auth.oidc.link_verified_email', '{value}', 4, NULL, {T0_MS})"
    );
    exec(conn, &sql).await.unwrap();
}

async fn via(conn: &DatabaseConnection, subject: &str) -> LinkedVia {
    user_identity::Entity::find()
        .filter(user_identity::Column::Subject.eq(subject))
        .one(conn)
        .await
        .unwrap()
        .unwrap()
        .linked_via
}

#[tokio::test]
async fn existing_identities_get_their_source_and_flows_survive() {
    each_backend(async |db: &TestDb| {
        migrate_before_m4_09(&db.conn).await;
        let provider = provider_row("corp").insert(&db.conn).await.unwrap().id;
        let other = provider_row("other").insert(&db.conn).await.unwrap().id;
        let olga = user(&db.conn, "olga", AuthSource::Oidc).await;
        let alice = user(&db.conn, "alice", AuthSource::Local).await;
        old_identity(db, olga, provider, "olga-1").await;
        old_identity(db, olga, other, "olga-2").await;
        old_identity(db, alice, provider, "alice-1").await;
        flow_row(provider, "state-hash", None)
            .insert(&db.conn)
            .await
            .unwrap();

        migrate_steps(&db.conn, None).await;

        let b = db.backend;
        assert_eq!(via(&db.conn, "olga-1").await, LinkedVia::Provisioned, "{b}");
        assert_eq!(via(&db.conn, "olga-2").await, LinkedVia::Email, "{b}");
        assert_eq!(via(&db.conn, "alice-1").await, LinkedVia::Email, "{b}");
        let flows = oidc_flow::Entity::find().all(&db.conn).await.unwrap();
        assert_eq!(flows.len(), 1, "{b}");
        assert_eq!(flows[0].state_hash, "state-hash", "{b}");
        let mut link = flow_row(provider, "link-hash", None);
        link.flow = Set(Flow::Link);
        link.insert(&db.conn).await.unwrap();
        // The state hash stays unique after the rebuild.
        assert!(
            flow_row(provider, "state-hash", None)
                .insert(&db.conn)
                .await
                .is_err(),
            "{b}"
        );
    })
    .await;
}

#[tokio::test]
async fn link_sources_and_flow_kinds_are_constrained() {
    each_backend(async |db: &TestDb| {
        migrate_steps(&db.conn, None).await;
        let provider = provider_row("corp").insert(&db.conn).await.unwrap().id;
        let olga = user(&db.conn, "olga", AuthSource::Oidc).await;
        common::identity_row(olga, provider, "s")
            .insert(&db.conn)
            .await
            .unwrap();
        flow_row(provider, "h", None)
            .insert(&db.conn)
            .await
            .unwrap();

        assert_checks(
            db,
            &[
                (
                    "user_identities",
                    "linked_via = 'manual'",
                    "linked_via = 'email'",
                ),
                ("oidc_flows", "flow = 'other'", "flow = 'link'"),
                // A link flow is no client flow: it has no polling code.
                (
                    "oidc_flows",
                    "client_code_hash = 'c'",
                    "client_code_hash = NULL",
                ),
            ],
        )
        .await;
    })
    .await;
}

async fn stored(conn: &DatabaseConnection, key: &str) -> Option<setting::Model> {
    setting::Entity::find_by_id(key.to_owned())
        .one(conn)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_email_link_switch_becomes_a_role_ceiling() {
    for (old, new) in [
        ("true", Some("\"user\"")),
        ("false", Some("\"off\"")),
        ("1", None),
    ] {
        each_backend(async |db: &TestDb| {
            migrate_before_m4_09(&db.conn).await;
            old_setting(&db.conn, old).await;

            migrate_steps(&db.conn, None).await;

            let b = db.backend;
            assert!(
                stored(&db.conn, "auth.oidc.link_verified_email")
                    .await
                    .is_none(),
                "{b}: {old}"
            );
            let row = stored(&db.conn, "auth.oidc.link_email_max_role").await;
            assert_eq!(row.as_ref().map(|r| r.value.as_str()), new, "{b}: {old}");
            let history = setting_history::Entity::find()
                .filter(setting_history::Column::Key.eq("auth.oidc.link_email_max_role"))
                .order_by_asc(setting_history::Column::Id)
                .all(&db.conn)
                .await
                .unwrap();
            assert_eq!(history.len(), usize::from(new.is_some()), "{b}: {old}");
            if let (Some(row), Some(entry)) = (row, history.first()) {
                assert_eq!((row.version, entry.version), (1, 1), "{b}");
                assert_eq!(row.updated_at, ts(T0_MS), "{b}");
                assert!(
                    entry
                        .reason
                        .as_deref()
                        .unwrap()
                        .contains("link_verified_email")
                );
            }
        })
        .await;
    }
}

#[tokio::test]
async fn nothing_stored_leaves_the_default() {
    each_backend(async |db: &TestDb| {
        migrate_steps(&db.conn, None).await;

        assert!(
            stored(&db.conn, "auth.oidc.link_email_max_role")
                .await
                .is_none(),
            "{}",
            db.backend
        );
    })
    .await;
}
