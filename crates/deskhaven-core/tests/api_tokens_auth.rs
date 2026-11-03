//! ApiTokenService checks (M2-19, 06 §3.2): scopes, malformed and unknown tokens, expiry,
//! the creator's status and role (read time, and durable revocation) and the throttled
//! `last_used_at`. Every test runs on SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::api_tokens::{Env, T0_MS};
use deskhaven_core::api_tokens::{API_TOKEN_PREFIX, ApiTokenError, NewApiToken, Scope};
use deskhaven_core::clock::Clock;
use deskhaven_core::users::{Role, Status};
use deskhaven_db::entity::{api_token, user};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};

const MINUTE: Duration = Duration::from_secs(60);

#[tokio::test]
async fn a_token_authenticates_for_its_scopes_only() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let admin = env.admin("root").await;
        let (id, raw) = env.issue(Some(admin), &[Scope::Assign]).await;

        // Act
        let granted = env.tokens.authenticate(&raw, Scope::Assign).await.unwrap();
        let denied = env.tokens.authenticate(&raw, Scope::Deploy).await;

        // Assert
        assert_eq!(granted.token_id, id, "{}", db.backend);
        assert_eq!(granted.created_by, Some(admin), "{}", db.backend);
        assert_eq!(
            granted.scopes,
            BTreeSet::from([Scope::Assign]),
            "{}",
            db.backend
        );
        let err = denied.unwrap_err();
        assert!(matches!(err, ApiTokenError::MissingScope), "{}", db.backend);
        assert!(err.is_unauthenticated(), "{}", db.backend);
    })
    .await;
}

#[tokio::test]
async fn unknown_and_malformed_tokens_do_not_authenticate() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, raw) = env.issue(None, &[Scope::Deploy]).await;
        let mut flipped = raw.clone();
        let last = if flipped.ends_with('A') { "B" } else { "A" };
        flipped.replace_range(flipped.len() - 1.., last);

        for bad in [
            String::new(),
            "dhk_".to_owned(),
            raw[API_TOKEN_PREFIX.len()..].to_owned(),
            format!("{raw}A"),
            raw.replacen("dhk_", "dhx_", 1),
            flipped,
        ] {
            let err = env
                .tokens
                .authenticate(&bad, Scope::Deploy)
                .await
                .unwrap_err();
            assert!(
                matches!(err, ApiTokenError::InvalidToken),
                "{}: {bad:?} {err:?}",
                db.backend
            );
            assert!(err.is_unauthenticated(), "{}", db.backend);
        }
    })
    .await;
}

#[tokio::test]
async fn expired_tokens_do_not_authenticate() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let new = NewApiToken {
            expires_at: Some(time::from_millis(T0_MS + 60_000).unwrap()),
            ..env.new_token(None, &[Scope::Deploy])
        };
        let (_, secret) = env.tokens.create(new).await.unwrap();

        env.tokens
            .authenticate(secret.expose(), Scope::Deploy)
            .await
            .unwrap();
        env.clock.advance(MINUTE);
        let late = env
            .tokens
            .authenticate(secret.expose(), Scope::Deploy)
            .await;

        assert!(
            matches!(late, Err(ApiTokenError::Expired)),
            "{}: {late:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn revoked_tokens_do_not_authenticate_and_revoke_is_idempotent() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let (id, raw) = env.issue(None, &[Scope::Deploy]).await;

        // Act
        let first = env.tokens.revoke(id).await.unwrap();
        env.clock.advance(MINUTE);
        let again = env.tokens.revoke(id).await.unwrap();
        let unknown = env.tokens.revoke(id + 100).await.unwrap();
        let result = env.tokens.authenticate(&raw, Scope::Deploy).await;

        // Assert
        assert!(first && again && !unknown, "{}", db.backend);
        assert_eq!(
            env.row(id).await.revoked_at,
            Some(time::from_millis(T0_MS).unwrap()),
            "{}: the first revocation time is kept",
            db.backend
        );
        assert!(
            matches!(result, Err(ApiTokenError::Revoked)),
            "{}: {result:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn a_token_dies_with_its_creator_or_their_admin_rights() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let disabled = env.admin("dora").await;
        let demoted = env.admin("dean").await;
        let deleted = env.admin("dell").await;
        let (_, t_disabled) = env.issue(Some(disabled), &[Scope::Deploy]).await;
        let (_, t_demoted) = env.issue(Some(demoted), &[Scope::Deploy]).await;
        let (id_deleted, _) = env.issue(Some(deleted), &[Scope::Deploy]).await;

        env.users
            .set_status(disabled, Status::Disabled)
            .await
            .unwrap();
        env.users.set_role(demoted, Role::User).await.unwrap();
        env.users.delete(deleted).await.unwrap();

        for raw in [t_disabled, t_demoted] {
            let result = env.tokens.authenticate(&raw, Scope::Deploy).await;
            assert!(
                matches!(result, Err(ApiTokenError::Revoked)),
                "{}: {result:?}",
                db.backend
            );
        }
        let gone = api_token::Entity::find_by_id(id_deleted)
            .one(&env.db)
            .await
            .unwrap();
        assert!(gone.is_none(), "{}: deleted with its creator", db.backend);
    })
    .await;
}

#[tokio::test]
async fn last_used_at_is_written_at_most_once_a_minute() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (id, raw) = env.issue(None, &[Scope::Deploy]).await;
        let at = |ms: i64| Some(time::from_millis(ms).unwrap());

        assert_eq!(env.row(id).await.last_used_at, None, "{}", db.backend);
        env.tokens.authenticate(&raw, Scope::Deploy).await.unwrap();
        assert_eq!(env.row(id).await.last_used_at, at(T0_MS), "{}", db.backend);
        env.clock.advance(Duration::from_secs(30));
        env.tokens.authenticate(&raw, Scope::Deploy).await.unwrap();
        assert_eq!(env.row(id).await.last_used_at, at(T0_MS), "{}", db.backend);
        env.clock.advance(Duration::from_secs(30));
        env.tokens.authenticate(&raw, Scope::Deploy).await.unwrap();
        let now = env.clock.now_ms();
        assert_eq!(env.row(id).await.last_used_at, at(now), "{}", db.backend);
    })
    .await;
}

/// 06 §3.2 "禁用时吊销": disabling the creator revokes their tokens for good, so
/// re-enabling a (possibly compromised) account resurrects none of them.
#[tokio::test]
async fn disabling_the_creator_revokes_their_tokens_durably() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let admin = env.admin("dora").await;
        let other = env.admin("otto").await;
        let (id, raw) = env.issue(Some(admin), &[Scope::Deploy]).await;
        let (other_id, _) = env.issue(Some(other), &[Scope::Deploy]).await;

        // Act
        env.users.set_status(admin, Status::Disabled).await.unwrap();
        env.users.set_status(admin, Status::Active).await.unwrap();
        let result = env.tokens.authenticate(&raw, Scope::Deploy).await;

        // Assert
        let b = db.backend;
        assert!(
            matches!(result, Err(ApiTokenError::Revoked)),
            "{b}: {result:?}"
        );
        assert!(env.row(id).await.revoked_at.is_some(), "{b}");
        let kept = env.row(other_id).await.revoked_at;
        assert!(kept.is_none(), "{b}: other creators keep theirs");
    })
    .await;
}

/// Same for losing the administrator role: promoting the account again resurrects nothing.
/// Owner -> admin keeps the tokens (still an administrator).
#[tokio::test]
async fn demoting_the_creator_to_user_revokes_their_tokens_durably() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let admin = env.admin("dean").await;
        let (id, raw) = env.issue(Some(admin), &[Scope::Assign]).await;

        // Act
        let owner = env.admin("olga").await;
        env.users.set_role(owner, Role::Owner).await.unwrap();
        env.users.set_role(admin, Role::Owner).await.unwrap();
        env.users.set_role(admin, Role::Admin).await.unwrap();
        let still = env.tokens.authenticate(&raw, Scope::Assign).await;
        env.users.set_role(admin, Role::User).await.unwrap();
        env.users.set_role(admin, Role::Admin).await.unwrap();
        let result = env.tokens.authenticate(&raw, Scope::Assign).await;

        // Assert
        let b = db.backend;
        assert!(still.is_ok(), "{b}: {still:?}");
        assert!(
            matches!(result, Err(ApiTokenError::Revoked)),
            "{b}: {result:?}"
        );
        assert!(env.row(id).await.revoked_at.is_some(), "{b}");
    })
    .await;
}

/// Defence in depth: a creator changed behind the service's back (no revocation written)
/// still stops their tokens at read time.
#[tokio::test]
async fn a_creator_disabled_out_of_band_stops_their_tokens_at_read_time() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let admin = env.admin("dora").await;
        let (_, raw) = env.issue(Some(admin), &[Scope::Deploy]).await;
        let disable = user::ActiveModel {
            id: Set(admin),
            status: Set(Status::Disabled),
            ..Default::default()
        };
        disable.update(&env.db).await.unwrap();

        let result = env.tokens.authenticate(&raw, Scope::Deploy).await;

        assert!(
            matches!(result, Err(ApiTokenError::CreatorInactive)),
            "{}: {result:?}",
            db.backend
        );
    })
    .await;
}
