//! Identity & access entities: insert + select round trips and relations, on every backend.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used)]

mod common;

use common::{
    T0_MS, api_token_row, challenge_row, flow_row, group_row, identity_row, ldap_row, member_row,
    provider_row, recovery_row, session_row, setup_token_row, ts, user_row, user_row_with_email,
};
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::user::{AuthSource, Role, Status};
use deskhaven_db::entity::{
    api_token, auth_challenge, group, group_member, ldap_directory, oidc_flow, oidc_provider,
    recovery_code, session, setup_token, user, user_identity,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::{ActiveModelTrait, ModelTrait, Set};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};

#[tokio::test]
async fn user_round_trips_every_column() {
    each_migrated_backend(async |db: &TestDb| {
        let mut row = user_row_with_email("Alice", "Alice@Example.com");
        row.role = Set(Role::Admin);
        row.status = Set(Status::Unverified);
        row.email_verified = Set(true);
        row.must_change_password = Set(true);
        row.totp_secret_enc = Set(Some("v1:k1:nonce:ct".to_owned()));
        row.totp_enabled_at = Set(Some(ts(T0_MS + 5)));
        row.totp_last_step = Set(Some(56_666_667));
        row.note = Set("note".to_owned());
        row.avatar_path = Set(Some("avatars/1.png".to_owned()));
        row.last_login_at = Set(Some(ts(T0_MS + 7)));

        let inserted = row.insert(&db.conn).await.unwrap();
        let found = user::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found, inserted, "{}", db.backend);
        assert_eq!(found.created_at.timestamp_millis(), T0_MS);
        assert_eq!(found.last_login_at, Some(ts(T0_MS + 7)));
        assert_eq!(found.email_norm.as_deref(), Some("alice@example.com"));
    })
    .await;
}

#[tokio::test]
async fn user_enums_round_trip_every_variant() {
    each_migrated_backend(async |db: &TestDb| {
        let roles = [Role::Owner, Role::Admin, Role::User];
        let statuses = [Status::Active, Status::Disabled, Status::Unverified];
        let sources = [AuthSource::Local, AuthSource::Ldap, AuthSource::Oidc];

        for (i, ((role, status), source)) in
            roles.into_iter().zip(statuses).zip(sources).enumerate()
        {
            let mut row = user_row(&format!("user{i}"));
            row.role = Set(role);
            row.status = Set(status);
            row.auth_source = Set(source);
            if source != AuthSource::Local {
                row.password_hash = Set(None);
            }
            let inserted = row.insert(&db.conn).await.unwrap();

            let found = user::Entity::find_by_id(inserted.id)
                .one(&db.conn)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (found.role, found.status, found.auth_source),
                (role, status, source)
            );
        }
    })
    .await;
}

#[tokio::test]
async fn timestamps_keep_millisecond_precision() {
    each_migrated_backend(async |db: &TestDb| {
        let now = time::now();
        let mut row = user_row("alice");
        row.created_at = Set(now);
        row.updated_at = Set(now);

        let inserted = row.insert(&db.conn).await.unwrap();
        let found = user::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found.created_at, now, "{}", db.backend);
        assert_eq!(found.updated_at.timestamp_millis(), now.timestamp_millis());
    })
    .await;
}

#[tokio::test]
async fn group_and_membership_round_trip() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let mut staff = group_row("staff");
        staff.managed_by = Set(ManagedBy::Config);
        let staff = staff.insert(conn).await.unwrap();
        let member = member_row(staff.id, alice.id).insert(conn).await.unwrap();

        let found_group = group::Entity::find_by_id(staff.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        let found_member = group_member::Entity::find_by_id((staff.id, alice.id))
            .one(conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found_group, staff, "{}", db.backend);
        assert_eq!(found_group.managed_by, ManagedBy::Config);
        assert_eq!(found_member, member);
    })
    .await;
}

#[tokio::test]
async fn provider_ldap_and_identity_round_trip() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let provider = provider_row("corp").insert(conn).await.unwrap();
        let ldap = ldap_row("dir").insert(conn).await.unwrap();
        let identity = identity_row(alice.id, provider.id, "sub-1")
            .insert(conn)
            .await
            .unwrap();

        let found_provider = oidc_provider::Entity::find_by_id(provider.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        let found_ldap = ldap_directory::Entity::find_by_id(ldap.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        let found_identity = user_identity::Entity::find_by_id(identity.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found_provider, provider, "{}", db.backend);
        assert_eq!(found_ldap, ldap);
        assert_eq!(found_identity, identity);
        assert!(found_ldap.tls_verify);
        assert_eq!(found_ldap.managed_by, ManagedBy::Config);
    })
    .await;
}

#[tokio::test]
async fn session_and_credentials_round_trip() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let mut console = session_row(alice.id, "console-hash");
        console.kind = Set(session::Kind::Console);
        console.revoked_at = Set(Some(ts(T0_MS + 9)));
        let session = console.insert(conn).await.unwrap();
        let token = api_token_row(Some(alice.id), "token-hash")
            .insert(conn)
            .await
            .unwrap();
        let recovery = recovery_row(alice.id, "recovery-hash")
            .insert(conn)
            .await
            .unwrap();
        let setup = setup_token_row("setup-hash").insert(conn).await.unwrap();

        let found_session = session::Entity::find_by_id(session.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        let found_token = api_token::Entity::find_by_id(token.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        let found_recovery = recovery_code::Entity::find_by_id(recovery.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        let found_setup = setup_token::Entity::find_by_id(setup.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found_session, session, "{}", db.backend);
        assert_eq!(found_session.kind, session::Kind::Console);
        assert_eq!(found_session.revoked_at, Some(ts(T0_MS + 9)));
        assert_eq!(found_token, token);
        assert_eq!(found_recovery, recovery);
        assert_eq!(found_setup, setup);
    })
    .await;
}

#[tokio::test]
async fn challenge_and_flow_round_trip() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let provider = provider_row("corp").insert(conn).await.unwrap();
        let mut totp = challenge_row(alice.id, "secret");
        totp.kind = Set(auth_challenge::Kind::Totp);
        totp.code_hash = Set(None);
        totp.attempts = Set(3);
        let challenge = totp.insert(conn).await.unwrap();
        let mut done = flow_row(provider.id, "state-hash", Some("code-hash"));
        done.completed_user_id = Set(Some(alice.id));
        done.error = Set(Some("access_denied".to_owned()));
        let flow = done.insert(conn).await.unwrap();

        let found_challenge = auth_challenge::Entity::find_by_id(challenge.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        let found_flow = oidc_flow::Entity::find_by_id(flow.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found_challenge, challenge, "{}", db.backend);
        assert_eq!(found_challenge.kind, auth_challenge::Kind::Totp);
        assert_eq!(found_challenge.attempts, 3);
        assert_eq!(found_flow, flow);
        assert_eq!(found_flow.completed_user_id, Some(alice.id));
        assert_eq!(found_flow.consumed_at, None);
    })
    .await;
}

#[tokio::test]
async fn public_client_provider_has_no_secret() {
    each_migrated_backend(async |db: &TestDb| {
        let mut row = provider_row("public");
        row.client_secret_enc = Set(None);

        let inserted = row.insert(&db.conn).await.unwrap();
        let found = oidc_provider::Entity::find_by_id(inserted.id)
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found.client_secret_enc, None, "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn flows_and_challenges_are_consumed_exactly_once() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let provider = provider_row("corp").insert(conn).await.unwrap();
        let flow = flow_row(provider.id, "state-hash", Some("code-hash"))
            .insert(conn)
            .await
            .unwrap();
        let challenge = challenge_row(alice.id, "secret-hash")
            .insert(conn)
            .await
            .unwrap();
        let now = ts(T0_MS + 1_000);

        let consume_flow = || async {
            oidc_flow::Entity::update_many()
                .col_expr(oidc_flow::Column::ConsumedAt, Expr::value(now))
                .filter(oidc_flow::Column::Id.eq(flow.id))
                .filter(oidc_flow::Column::ConsumedAt.is_null())
                .exec(conn)
                .await
                .unwrap()
                .rows_affected
        };
        let consume_challenge = || async {
            auth_challenge::Entity::update_many()
                .col_expr(auth_challenge::Column::ConsumedAt, Expr::value(now))
                .filter(auth_challenge::Column::Id.eq(challenge.id))
                .filter(auth_challenge::Column::ConsumedAt.is_null())
                .exec(conn)
                .await
                .unwrap()
                .rows_affected
        };

        assert_eq!(consume_flow().await, 1, "{}", db.backend);
        assert_eq!(consume_flow().await, 0, "{}", db.backend);
        assert_eq!(consume_challenge().await, 1, "{}", db.backend);
        assert_eq!(consume_challenge().await, 0, "{}", db.backend);
        let stored = oidc_flow::Entity::find_by_id(flow.id)
            .one(conn)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.consumed_at, Some(now));
    })
    .await;
}

#[tokio::test]
async fn relations_navigate_both_ways() {
    each_migrated_backend(async |db: &TestDb| {
        let conn = &db.conn;
        let alice = user_row("alice").insert(conn).await.unwrap();
        let staff = group_row("staff").insert(conn).await.unwrap();
        let ops = group_row("ops").insert(conn).await.unwrap();
        member_row(staff.id, alice.id).insert(conn).await.unwrap();
        member_row(ops.id, alice.id).insert(conn).await.unwrap();
        session_row(alice.id, "s1").insert(conn).await.unwrap();
        session_row(alice.id, "s2").insert(conn).await.unwrap();
        let token = api_token_row(Some(alice.id), "t")
            .insert(conn)
            .await
            .unwrap();

        let groups = alice.find_related(group::Entity).all(conn).await.unwrap();
        let sessions = alice.find_related(session::Entity).all(conn).await.unwrap();
        let members = staff.find_related(user::Entity).all(conn).await.unwrap();
        let creator = token.find_related(user::Entity).one(conn).await.unwrap();

        assert_eq!(groups.len(), 2, "{}", db.backend);
        assert_eq!(sessions.len(), 2);
        assert_eq!(members, vec![alice.clone()]);
        assert_eq!(creator, Some(alice));
    })
    .await;
}
