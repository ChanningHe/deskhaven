//! Identity & access schema (M2-01): uniqueness, CHECK and foreign-key behavior, on every
//! available backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::{
    api_token_row, challenge_row, count, exec, flow_row, group_row, identity_row, ldap_row,
    member_row, provider_row, recovery_row, session_row, setup_token_row, user_row,
    user_row_with_email,
};
use deskhaven_db::entity::{
    api_token, auth_challenge, group, group_member, oidc_flow, oidc_provider, recovery_code,
    session, user, user_identity,
};
use deskhaven_db::testing::{TestDb, each_backend, each_migrated_backend};
use deskhaven_db::{migrate, migration_status};
use sea_orm::{ActiveModelTrait, DatabaseConnection, DbErr, EntityTrait, SqlErr};
use sea_orm_migration::prelude::*;

const IDENTITY_TABLES: [&str; 12] = [
    "users",
    "groups",
    "group_members",
    "user_identities",
    "oidc_providers",
    "ldap_directories",
    "sessions",
    "api_tokens",
    "recovery_codes",
    "auth_challenges",
    "oidc_flows",
    "setup_tokens",
];

fn is_unique_violation(res: Result<impl Sized, DbErr>) -> bool {
    matches!(
        res.err().and_then(|e| e.sql_err()),
        Some(SqlErr::UniqueConstraintViolation(_))
    )
}

fn is_fk_violation(res: Result<impl Sized, DbErr>) -> bool {
    let Some(err) = res.err() else { return false };
    // SQLite reports ON DELETE RESTRICT with extended code 1811, which sea-orm does not
    // classify; the message is the same as for any other foreign-key failure.
    matches!(
        err.sql_err(),
        Some(SqlErr::ForeignKeyConstraintViolation(_))
    ) || err.to_string().contains("FOREIGN KEY constraint failed")
}

async fn has_index(conn: &DatabaseConnection, table: &str, index: &str) -> bool {
    SchemaManager::new(conn)
        .has_index(table, index)
        .await
        .unwrap()
}

#[tokio::test]
async fn migration_creates_identity_tables_and_is_idempotent() {
    each_backend(async |db: &TestDb| {
        migrate(&db.conn).await.unwrap();
        migrate(&db.conn).await.unwrap();

        let manager = SchemaManager::new(&db.conn);
        for table in IDENTITY_TABLES {
            assert!(
                manager.has_table(table).await.unwrap(),
                "{}: missing {table}",
                db.backend
            );
        }
        let status = migration_status(&db.conn).await.unwrap();
        assert!(status.is_current(), "{}: {status:?}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn lookup_and_cleanup_indexes_exist() {
    each_migrated_backend(async |db: &TestDb| {
        let expected = [
            ("users", "uq_users_username_norm"),
            ("users", "uq_users_email_norm"),
            ("sessions", "uq_sessions_token_hash"),
            ("sessions", "idx_sessions_user_id"),
            ("sessions", "idx_sessions_expires_at"),
            ("api_tokens", "uq_api_tokens_token_hash"),
            ("sessions", "idx_sessions_last_used_at"),
            ("group_members", "idx_group_members_user_id"),
            ("user_identities", "idx_user_identities_user_id"),
            ("api_tokens", "idx_api_tokens_created_by"),
            ("recovery_codes", "uq_recovery_codes_user_code"),
            ("auth_challenges", "idx_auth_challenges_user_id"),
            ("oidc_flows", "idx_oidc_flows_provider_id"),
            ("oidc_flows", "idx_oidc_flows_completed_user_id"),
            ("ldap_directories", "uq_ldap_directories_enabled"),
            ("auth_challenges", "uq_auth_challenges_secret_hash"),
            ("auth_challenges", "idx_auth_challenges_expires_at"),
            ("oidc_flows", "uq_oidc_flows_state_hash"),
            ("oidc_flows", "uq_oidc_flows_client_code_hash"),
            ("oidc_flows", "idx_oidc_flows_expires_at"),
            ("setup_tokens", "idx_setup_tokens_expires_at"),
            ("user_identities", "uq_user_identities_provider_subject"),
        ];
        for (table, index) in expected {
            assert!(
                has_index(&db.conn, table, index).await,
                "{}: missing {index} on {table}",
                db.backend
            );
        }
    })
    .await;
}

#[tokio::test]
async fn username_norm_is_case_insensitively_unique() {
    each_migrated_backend(async |db: &TestDb| {
        user_row("Alice").insert(&db.conn).await.unwrap();

        // Same normalized form, different spelling.
        let res = user_row("ALICE").insert(&db.conn).await;

        assert!(is_unique_violation(res), "{}", db.backend);
        assert_eq!(count::<user::Entity>(&db.conn).await, 1);
    })
    .await;
}

#[tokio::test]
async fn stale_username_norm_is_rejected() {
    each_migrated_backend(async |db: &TestDb| {
        let mut row = user_row("Alice");
        row.username_norm = sea_orm::Set("Alice".to_owned());

        let res = row.insert(&db.conn).await;

        assert!(res.is_err(), "{}: un-normalized username_norm", db.backend);
    })
    .await;
}

#[tokio::test]
async fn email_norm_is_case_insensitively_unique() {
    each_migrated_backend(async |db: &TestDb| {
        user_row_with_email("alice", "Alice@Example.com")
            .insert(&db.conn)
            .await
            .unwrap();

        let res = user_row_with_email("bob", "ALICE@example.COM")
            .insert(&db.conn)
            .await;

        assert!(is_unique_violation(res), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn null_emails_do_not_conflict() {
    each_migrated_backend(async |db: &TestDb| {
        user_row("alice").insert(&db.conn).await.unwrap();
        user_row("bob").insert(&db.conn).await.unwrap();
        user_row("carol").insert(&db.conn).await.unwrap();

        assert_eq!(count::<user::Entity>(&db.conn).await, 3, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn email_and_email_norm_must_be_set_together() {
    each_migrated_backend(async |db: &TestDb| {
        let mut email_only = user_row("alice");
        email_only.email = sea_orm::Set(Some("a@example.com".to_owned()));
        let mut norm_only = user_row("bob");
        norm_only.email_norm = sea_orm::Set(Some("b@example.com".to_owned()));

        assert!(email_only.insert(&db.conn).await.is_err(), "{}", db.backend);
        assert!(norm_only.insert(&db.conn).await.is_err(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn usernames_are_limited_to_the_ascii_charset() {
    each_migrated_backend(async |db: &TestDb| {
        for rejected in ["é", "名前", "al ice", "a/b", "a'b", "a\nb", "ａｌｉｃｅ"] {
            let res = user_row(rejected).insert(&db.conn).await;
            assert!(res.is_err(), "{}: accepted {rejected:?}", db.backend);
        }
        user_row("Al.ice_9@corp-x").insert(&db.conn).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn empty_email_norm_is_rejected() {
    each_migrated_backend(async |db: &TestDb| {
        let res = user_row_with_email("alice", "").insert(&db.conn).await;

        assert!(res.is_err(), "{}", db.backend);
    })
    .await;
}

/// `(table, bad assignment, good assignment)` run as `UPDATE <table> SET ...` on a seeded row.
/// The good assignment is a control: the same statement shape succeeds with a valid value.
/// Cases run in order on the same rows, so a good assignment may change the row for later cases.
fn check_cases() -> Vec<(&'static str, String, String)> {
    let long_name = "a".repeat(65);
    let case = |table, bad: &str, good: &str| (table, bad.to_owned(), good.to_owned());
    vec![
        case("users", "role = 'root'", "role = 'admin'"),
        case("users", "status = 'banned'", "status = 'disabled'"),
        case("users", "auth_source = 'saml'", "auth_source = 'local'"),
        case(
            "users",
            "username = '', username_norm = ''",
            "username = 'x', username_norm = 'x'",
        ),
        case(
            "users",
            &format!("username = '{long_name}', username_norm = '{long_name}'"),
            "username = 'Alice', username_norm = 'alice'",
        ),
        case(
            "users",
            "username_norm = 'other'",
            "username_norm = 'alice'",
        ),
        case(
            "users",
            "email = 'a@example.com'",
            "email = 'a@example.com', email_norm = 'a@example.com'",
        ),
        case(
            "users",
            "auth_source = 'ldap'",
            "auth_source = 'ldap', password_hash = NULL",
        ),
        case(
            "users",
            "totp_enabled_at = 1",
            "totp_enabled_at = 1, totp_secret_enc = 'v1:k:n:c'",
        ),
        case("groups", "managed_by = 'both'", "managed_by = 'config'"),
        case("oidc_providers", "managed_by = 'both'", "managed_by = 'ui'"),
        case(
            "ldap_directories",
            "managed_by = 'both'",
            "managed_by = 'ui'",
        ),
        case(
            "users",
            "username = 'é', username_norm = 'é'",
            "username = 'Alice', username_norm = 'alice'",
        ),
        case(
            "users",
            "email = '', email_norm = ''",
            "email = 'a@example.com', email_norm = 'a@example.com'",
        ),
        // SQLite stores booleans as integers; PostgreSQL rejects the integer on type.
        case("users", "email_verified = 2", "email_verified = true"),
        case(
            "users",
            "must_change_password = 2",
            "must_change_password = true",
        ),
        case("oidc_providers", "enabled = 2", "enabled = false"),
        case("ldap_directories", "start_tls = 2", "start_tls = true"),
        // The seeded session is a client session bound to a device.
        case("sessions", "kind = 'api'", "kind = 'client'"),
        case("sessions", "rustdesk_id = NULL", "rustdesk_id = '42'"),
        case("sessions", "device_uuid = NULL", "device_uuid = 'u2'"),
        case(
            "sessions",
            "kind = 'client', device_uuid = NULL",
            "kind = 'console', device_uuid = NULL, rustdesk_id = NULL",
        ),
        // The seeded challenge is an email_code with a code hash.
        case("auth_challenges", "code_hash = NULL", "code_hash = 'h2'"),
        case(
            "auth_challenges",
            "kind = 'sms'",
            "kind = 'totp', code_hash = NULL",
        ),
        case("auth_challenges", "code_hash = 'x'", "attempts = 2"),
        case("auth_challenges", "attempts = -1", "attempts = 5"),
        // The seeded flow is a console flow without a client code.
        case("oidc_flows", "flow = 'mobile'", "flow = 'console'"),
        case(
            "oidc_flows",
            "client_code_hash = 'x'",
            "flow = 'client', client_code_hash = 'x'",
        ),
        case(
            "oidc_flows",
            "client_code_hash = NULL",
            "flow = 'console', client_code_hash = NULL",
        ),
    ]
}

#[tokio::test]
async fn check_constraints_reject_invalid_values() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("Alice").insert(conn).await.unwrap();
        group_row("staff").insert(conn).await.unwrap();
        let provider = provider_row("corp").insert(conn).await.unwrap();
        ldap_row("dir").insert(conn).await.unwrap();
        session_row(alice.id, "s1").insert(conn).await.unwrap();
        challenge_row(alice.id, "c1").insert(conn).await.unwrap();
        flow_row(provider.id, "f1", None)
            .insert(conn)
            .await
            .unwrap();

        for (table, bad, good) in check_cases() {
            let bad_sql = format!("UPDATE {table} SET {bad}");
            let good_sql = format!("UPDATE {table} SET {good}");
            assert!(
                exec(conn, &bad_sql).await.is_err(),
                "{}: accepted: {bad_sql}",
                db.backend
            );
            exec(conn, &good_sql)
                .await
                .unwrap_or_else(|e| panic!("{}: control rejected: {good_sql}: {e}", db.backend));
        }
    })
    .await;
}

#[tokio::test]
async fn deleting_a_user_cascades_to_owned_rows_and_created_tokens() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let bob = user_row("bob").insert(conn).await.unwrap();
        let staff = group_row("staff").insert(conn).await.unwrap();
        let provider = provider_row("corp").insert(conn).await.unwrap();
        member_row(staff.id, alice.id).insert(conn).await.unwrap();
        member_row(staff.id, bob.id).insert(conn).await.unwrap();
        identity_row(alice.id, provider.id, "sub-a")
            .insert(conn)
            .await
            .unwrap();
        identity_row(bob.id, provider.id, "sub-b")
            .insert(conn)
            .await
            .unwrap();
        session_row(alice.id, "sa").insert(conn).await.unwrap();
        session_row(bob.id, "sb").insert(conn).await.unwrap();
        recovery_row(alice.id, "ra").insert(conn).await.unwrap();
        recovery_row(bob.id, "rb").insert(conn).await.unwrap();
        challenge_row(alice.id, "ca").insert(conn).await.unwrap();
        challenge_row(bob.id, "cb").insert(conn).await.unwrap();
        api_token_row(Some(alice.id), "ta")
            .insert(conn)
            .await
            .unwrap();
        api_token_row(Some(bob.id), "tb")
            .insert(conn)
            .await
            .unwrap();
        api_token_row(None, "t-orphan").insert(conn).await.unwrap();
        let mut completed = flow_row(provider.id, "fa", Some("ca"));
        completed.completed_user_id = sea_orm::Set(Some(alice.id));
        completed.insert(conn).await.unwrap();
        let mut other = flow_row(provider.id, "fb", Some("cb"));
        other.completed_user_id = sea_orm::Set(Some(bob.id));
        other.insert(conn).await.unwrap();

        user::Entity::delete_by_id(alice.id)
            .exec(conn)
            .await
            .unwrap();

        let b = db.backend;
        assert_eq!(count::<group_member::Entity>(conn).await, 1, "{b}: members");
        assert_eq!(
            count::<user_identity::Entity>(conn).await,
            1,
            "{b}: identities"
        );
        assert_eq!(count::<session::Entity>(conn).await, 1, "{b}: sessions");
        assert_eq!(
            count::<recovery_code::Entity>(conn).await,
            1,
            "{b}: recovery"
        );
        assert_eq!(
            count::<auth_challenge::Entity>(conn).await,
            1,
            "{b}: challenges"
        );
        assert_eq!(count::<group::Entity>(conn).await, 1, "{b}: group kept");
        assert_eq!(count::<oidc_flow::Entity>(conn).await, 1, "{b}: flows");
        // Tokens die with their creator; others (and creator-less ones) stay.
        let tokens = api_token::Entity::find().all(conn).await.unwrap();
        let mut creators: Vec<_> = tokens.iter().map(|t| t.created_by).collect();
        creators.sort();
        assert_eq!(creators, vec![None, Some(bob.id)], "{b}: tokens");
    })
    .await;
}

#[tokio::test]
async fn deleting_a_group_cascades_and_a_provider_is_restricted_by_identities() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let staff = group_row("staff").insert(conn).await.unwrap();
        let provider = provider_row("corp").insert(conn).await.unwrap();
        member_row(staff.id, alice.id).insert(conn).await.unwrap();
        let identity = identity_row(alice.id, provider.id, "sub")
            .insert(conn)
            .await
            .unwrap();
        flow_row(provider.id, "state-1", Some("code-1"))
            .insert(conn)
            .await
            .unwrap();
        let b = db.backend;

        group::Entity::delete_by_id(staff.id)
            .exec(conn)
            .await
            .unwrap();
        assert_eq!(count::<group_member::Entity>(conn).await, 0, "{b}: members");
        assert_eq!(count::<user::Entity>(conn).await, 1, "{b}: user kept");

        // A linked identity blocks deleting its provider; nothing is removed.
        let blocked = oidc_provider::Entity::delete_by_id(provider.id)
            .exec(conn)
            .await;
        assert!(is_fk_violation(blocked), "{b}: provider with identities");
        assert_eq!(
            count::<oidc_provider::Entity>(conn).await,
            1,
            "{b}: provider"
        );
        assert_eq!(count::<oidc_flow::Entity>(conn).await, 1, "{b}: flows");

        // Without identities the provider goes, taking its in-flight flows along.
        user_identity::Entity::delete_by_id(identity.id)
            .exec(conn)
            .await
            .unwrap();
        oidc_provider::Entity::delete_by_id(provider.id)
            .exec(conn)
            .await
            .unwrap();
        assert_eq!(count::<oidc_flow::Entity>(conn).await, 0, "{b}: flows");
        assert_eq!(count::<user::Entity>(conn).await, 1, "{b}: user kept");
    })
    .await;
}

#[tokio::test]
async fn foreign_keys_reject_dangling_references() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let missing = 9_999;

        let session = session_row(missing, "s").insert(conn).await;
        let member = member_row(missing, missing).insert(conn).await;
        let flow = flow_row(missing, "st", None).insert(conn).await;
        let token = api_token_row(Some(missing), "t").insert(conn).await;
        let provider = provider_row("corp").insert(conn).await.unwrap();
        let mut completed = flow_row(provider.id, "st2", None);
        completed.completed_user_id = sea_orm::Set(Some(missing));
        let completed = completed.insert(conn).await;
        let identity = identity_row(missing, missing, "sub").insert(conn).await;
        let recovery = recovery_row(missing, "r").insert(conn).await;
        let challenge = challenge_row(missing, "c").insert(conn).await;

        assert!(is_fk_violation(completed), "{}: flow user", db.backend);
        assert!(is_fk_violation(identity), "{}: identity", db.backend);
        assert!(is_fk_violation(recovery), "{}: recovery", db.backend);
        assert!(is_fk_violation(challenge), "{}: challenge", db.backend);
        assert!(is_fk_violation(session), "{}: session", db.backend);
        assert!(is_fk_violation(member), "{}: member", db.backend);
        assert!(is_fk_violation(flow), "{}: flow", db.backend);
        assert!(is_fk_violation(token), "{}: token", db.backend);
    })
    .await;
}

#[tokio::test]
async fn token_hashes_and_handles_are_unique() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let b = db.backend;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let provider = provider_row("corp").insert(conn).await.unwrap();
        session_row(alice.id, "dup").insert(conn).await.unwrap();
        api_token_row(None, "dup").insert(conn).await.unwrap();
        challenge_row(alice.id, "dup").insert(conn).await.unwrap();
        flow_row(provider.id, "dup", Some("dup"))
            .insert(conn)
            .await
            .unwrap();
        setup_token_row("dup").insert(conn).await.unwrap();
        identity_row(alice.id, provider.id, "dup")
            .insert(conn)
            .await
            .unwrap();
        let group = group_row("dup").insert(conn).await.unwrap();
        disabled_ldap_row("dup").insert(conn).await.unwrap();
        member_row(group.id, alice.id).insert(conn).await.unwrap();
        recovery_row(alice.id, "dup").insert(conn).await.unwrap();

        assert!(
            is_unique_violation(session_row(alice.id, "dup").insert(conn).await),
            "{b}: session"
        );
        assert!(
            is_unique_violation(api_token_row(None, "dup").insert(conn).await),
            "{b}: api token"
        );
        assert!(
            is_unique_violation(challenge_row(alice.id, "dup").insert(conn).await),
            "{b}: secret"
        );
        assert!(
            is_unique_violation(flow_row(provider.id, "dup", None).insert(conn).await),
            "{b}: state"
        );
        assert!(
            is_unique_violation(
                flow_row(provider.id, "other", Some("dup"))
                    .insert(conn)
                    .await
            ),
            "{b}: client_code"
        );
        assert!(
            is_unique_violation(setup_token_row("dup").insert(conn).await),
            "{b}: setup token"
        );
        assert!(
            is_unique_violation(
                identity_row(alice.id, provider.id, "dup")
                    .insert(conn)
                    .await
            ),
            "{b}: identity"
        );
        assert!(
            is_unique_violation(group_row("dup").insert(conn).await),
            "{b}: group name"
        );
        assert!(
            is_unique_violation(disabled_ldap_row("dup").insert(conn).await),
            "{b}: ldap name"
        );
        assert!(
            is_unique_violation(recovery_row(alice.id, "dup").insert(conn).await),
            "{b}: recovery code"
        );
        assert!(
            is_unique_violation(provider_row("corp").insert(conn).await),
            "{b}: provider name"
        );
        assert!(
            is_unique_violation(member_row(group.id, alice.id).insert(conn).await),
            "{b}: membership"
        );
    })
    .await;
}

fn disabled_ldap_row(name: &str) -> deskhaven_db::entity::ldap_directory::ActiveModel {
    deskhaven_db::entity::ldap_directory::ActiveModel {
        enabled: sea_orm::Set(false),
        ..ldap_row(name)
    }
}

#[tokio::test]
async fn recovery_code_hash_may_repeat_across_users() {
    each_migrated_backend(async |db: &TestDb| {
        let alice = user_row("alice").insert(&db.conn).await.unwrap();
        let bob = user_row("bob").insert(&db.conn).await.unwrap();

        recovery_row(alice.id, "same")
            .insert(&db.conn)
            .await
            .unwrap();
        recovery_row(bob.id, "same").insert(&db.conn).await.unwrap();

        assert_eq!(
            count::<recovery_code::Entity>(&db.conn).await,
            2,
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn at_most_one_ldap_directory_is_enabled() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let b = db.backend;
        let first = ldap_row("one").insert(conn).await.unwrap();

        let second_enabled = ldap_row("two").insert(conn).await;
        disabled_ldap_row("two").insert(conn).await.unwrap();
        disabled_ldap_row("three").insert(conn).await.unwrap();

        assert!(is_unique_violation(second_enabled), "{b}: second enabled");
        // Enabling another directory requires disabling the first one.
        let enable = exec(
            conn,
            "UPDATE ldap_directories SET enabled = true WHERE name = 'two'",
        )
        .await;
        assert!(enable.is_err(), "{b}: enable while another is enabled");
        exec(
            conn,
            &format!(
                "UPDATE ldap_directories SET enabled = false WHERE id = {}",
                first.id
            ),
        )
        .await
        .unwrap();
        exec(
            conn,
            "UPDATE ldap_directories SET enabled = true WHERE name = 'two'",
        )
        .await
        .unwrap();
    })
    .await;
}

#[tokio::test]
async fn duplicate_group_membership_is_rejected() {
    each_migrated_backend(async |db: &TestDb| {
        let alice = user_row("alice").insert(&db.conn).await.unwrap();
        let staff = group_row("staff").insert(&db.conn).await.unwrap();
        member_row(staff.id, alice.id)
            .insert(&db.conn)
            .await
            .unwrap();

        let dup = member_row(staff.id, alice.id).insert(&db.conn).await;

        assert!(is_unique_violation(dup), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn null_client_codes_do_not_conflict() {
    each_migrated_backend(async |db: &TestDb| {
        let provider = provider_row("corp").insert(&db.conn).await.unwrap();

        flow_row(provider.id, "s1", None)
            .insert(&db.conn)
            .await
            .unwrap();
        flow_row(provider.id, "s2", None)
            .insert(&db.conn)
            .await
            .unwrap();

        assert_eq!(
            count::<oidc_flow::Entity>(&db.conn).await,
            2,
            "{}",
            db.backend
        );
    })
    .await;
}
