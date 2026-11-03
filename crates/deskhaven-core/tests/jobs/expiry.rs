//! Expiry cleanup: audit nonces after 24 h, finished setup tokens, strategy keys of devices
//! deleted longer than the grace period, and the services' own cleanups (sessions, OIDC
//! flows, login challenges, lockout counters), all on the manual clock.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::time::Duration;

use deskhaven_core::devices::DeviceRepository;
use deskhaven_core::jobs::{ExpiryCleanup, NONCE_RETENTION, Purge, STRATEGY_KEYS_GRACE};
use deskhaven_db::entity::{
    audit_nonce, auth_challenge, auth_lockout, device_strategy_key, oidc_flow, oidc_provider,
    session, setup_token, user,
};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};

use super::support::{Ctx, DAY, HOUR, T0_MS, ago, at, ctx};

fn cleanup(c: &Ctx) -> ExpiryCleanup {
    ExpiryCleanup::new(c.state.clone()).with_purge(Purge::new(2, Duration::ZERO))
}

async fn keys(c: &Ctx) -> Vec<String> {
    let mut ids: Vec<String> = device_strategy_key::Entity::find()
        .all(&c.conn)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.rustdesk_id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn nonces_are_kept_for_a_day() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange: more than two batches of expired nonces, one fresh.
        let c = ctx(db).await;
        for n in 0..5 {
            c.nonce(&format!("old-{n}"), ago(NONCE_RETENTION + HOUR))
                .await;
        }
        c.nonce("fresh", ago(NONCE_RETENTION - HOUR)).await;

        // Act
        let report = cleanup(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.audit_nonces, 5, "{b}");
        assert_eq!(report.failed, 0, "{b}");
        let left = audit_nonce::Entity::find().all(&c.conn).await.unwrap();
        assert_eq!(left.len(), 1, "{b}");
        assert_eq!(left[0].nonce, "fresh", "{b}");
    })
    .await;
}

async fn setup_row(c: &Ctx, hash: &str, expires_at: TimestampMs, used: bool) {
    setup_token::ActiveModel {
        token_hash: Set(hash.to_owned()),
        created_at: Set(ago(HOUR)),
        expires_at: Set(expires_at),
        used_at: Set(used.then(|| ago(HOUR))),
        ..Default::default()
    }
    .insert(&c.conn)
    .await
    .unwrap();
}

#[tokio::test]
async fn used_and_expired_setup_tokens_go_the_live_one_stays() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        setup_row(&c, "used", at(T0_MS + 1_000), true).await;
        setup_row(&c, "expired", at(T0_MS), false).await;
        setup_row(&c, "live", at(T0_MS + 1_000), false).await;

        // Act
        let report = cleanup(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.setup_tokens, 2, "{b}");
        let left = setup_token::Entity::find().all(&c.conn).await.unwrap();
        let hashes: Vec<&str> = left.iter().map(|r| r.token_hash.as_str()).collect();
        assert_eq!(hashes, ["live"], "{b}");
    })
    .await;
}

#[tokio::test]
async fn strategy_keys_go_only_after_their_device_is_gone_for_the_grace_period() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let past_grace = ago(STRATEGY_KEYS_GRACE + HOUR);
        c.register("111111111").await;
        c.strategy_keys("111111111", past_grace).await; // device exists
        c.strategy_keys("222222222", past_grace).await; // gone long enough
        c.strategy_keys("333333333", ago(STRATEGY_KEYS_GRACE - HOUR))
            .await; // gone recently

        // Act
        let report = cleanup(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.strategy_keys, 1, "{b}");
        assert_eq!(keys(&c).await, ["111111111", "333333333"], "{b}");
    })
    .await;
}

#[tokio::test]
async fn deleting_a_device_restarts_the_grace_period_of_its_keys() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange: keys last written long ago, device deleted now (wall clock: the stamp
        // shares `device_strategy_keys`' own clock).
        let c = ctx(db).await;
        c.register("111111111").await;
        c.strategy_keys("111111111", at(0)).await;
        let device = DeviceRepository::new(c.conn.clone())
            .find_by_rustdesk_id("111111111")
            .await
            .unwrap()
            .unwrap()
            .id;
        let before_delete = time::now().timestamp_millis();
        c.state.devices.delete(device).await.unwrap();
        c.clock.set(time::now().timestamp_millis() + 1_000);

        // Act
        let report = cleanup(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.strategy_keys, 0, "{b}");
        let row = device_strategy_key::Entity::find_by_id("111111111".to_owned())
            .one(&c.conn)
            .await
            .unwrap()
            .unwrap();
        assert!(row.updated_at.timestamp_millis() >= before_delete, "{b}");
    })
    .await;
}

/// A local user row, for the tables that reference one.
async fn user_row(c: &Ctx) -> i64 {
    user::ActiveModel {
        username: Set("alice".to_owned()),
        username_norm: Set("alice".to_owned()),
        display_name: Set("Alice".to_owned()),
        email_verified: Set(false),
        auth_source: Set(user::AuthSource::Local),
        role: Set(user::Role::User),
        status: Set(user::Status::Active),
        must_change_password: Set(false),
        note: Set(String::new()),
        created_at: Set(ago(DAY)),
        updated_at: Set(ago(DAY)),
        ..Default::default()
    }
    .insert(&c.conn)
    .await
    .unwrap()
    .id
}

async fn expired_session(c: &Ctx, user_id: i64) {
    session::ActiveModel {
        token_hash: Set("session-hash".to_owned()),
        kind: Set(session::Kind::Console),
        user_id: Set(user_id),
        created_at: Set(ago(2 * DAY)),
        last_used_at: Set(ago(HOUR)),
        expires_at: Set(ago(Duration::from_secs(1))),
        ..Default::default()
    }
    .insert(&c.conn)
    .await
    .unwrap();
}

async fn expired_challenge(c: &Ctx, user_id: i64) {
    auth_challenge::ActiveModel {
        secret_hash: Set("challenge-hash".to_owned()),
        user_id: Set(user_id),
        kind: Set(auth_challenge::Kind::Totp),
        attempts: Set(0),
        context: Set("{}".to_owned()),
        created_at: Set(ago(HOUR)),
        expires_at: Set(ago(Duration::from_secs(1))),
        ..Default::default()
    }
    .insert(&c.conn)
    .await
    .unwrap();
}

async fn expired_flow(c: &Ctx) {
    let provider = oidc_provider::ActiveModel {
        name: Set("idp".to_owned()),
        display_name: Set("IdP".to_owned()),
        issuer: Set("https://idp.example".to_owned()),
        client_id: Set("client".to_owned()),
        scopes: Set("openid".to_owned()),
        auto_provision: Set(false),
        enabled: Set(true),
        managed_by: Set(deskhaven_db::entity::managed_by::ManagedBy::Ui),
        created_at: Set(ago(DAY)),
        updated_at: Set(ago(DAY)),
        ..Default::default()
    }
    .insert(&c.conn)
    .await
    .unwrap();
    oidc_flow::ActiveModel {
        state_hash: Set("state-hash".to_owned()),
        provider_id: Set(provider.id),
        pkce_verifier_enc: Set("sealed".to_owned()),
        nonce: Set("nonce".to_owned()),
        flow: Set(oidc_flow::Flow::Console),
        context: Set("{}".to_owned()),
        created_at: Set(ago(HOUR)),
        expires_at: Set(ago(Duration::from_secs(1))),
        ..Default::default()
    }
    .insert(&c.conn)
    .await
    .unwrap();
}

async fn stale_lockout(c: &Ctx) {
    auth_lockout::ActiveModel {
        key: Set("user:alice".to_owned()),
        failures: Set(1),
        window_started_at: Set(ago(3 * DAY)),
        locked_until: Set(None),
        lock_level: Set(0),
        updated_at: Set(ago(3 * DAY)),
        last_success_at: Set(None),
    }
    .insert(&c.conn)
    .await
    .unwrap();
}

#[tokio::test]
async fn the_services_own_cleanups_run_too() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let c = ctx(db).await;
        let alice = user_row(&c).await;
        expired_session(&c, alice).await;
        expired_challenge(&c, alice).await;
        expired_flow(&c).await;
        stale_lockout(&c).await;

        // Act
        let report = cleanup(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.sessions, 1, "{b}");
        assert_eq!(report.auth_challenges, 1, "{b}");
        assert_eq!(report.oidc_flows, 1, "{b}");
        assert_eq!(report.auth_lockouts, 1, "{b}");
        assert_eq!(report.failed, 0, "{b}");
        assert_eq!(c.count(session::Entity).await, 0, "{b}");
        assert_eq!(c.count(auth_challenge::Entity).await, 0, "{b}");
        assert_eq!(c.count(oidc_flow::Entity).await, 0, "{b}");
        assert_eq!(c.count(auth_lockout::Entity).await, 0, "{b}");
    })
    .await;
}

#[tokio::test]
async fn a_failing_step_is_counted_and_the_others_still_run() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange: the setup token table is gone, the nonces are still there.
        let c = ctx(db).await;
        c.nonce("old", ago(NONCE_RETENTION + HOUR)).await;
        sea_orm::ConnectionTrait::execute_unprepared(&c.conn, "DROP TABLE setup_tokens")
            .await
            .unwrap();

        // Act
        let report = cleanup(&c).run().await;

        // Assert
        let b = db.backend;
        assert_eq!(report.failed, 1, "{b}");
        assert_eq!(report.audit_nonces, 1, "{b}");
    })
    .await;
}
