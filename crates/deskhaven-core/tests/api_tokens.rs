//! ApiTokenService issuing (M2-19, 06 §3.2): `dhk_` tokens from the CSPRNG, only their
//! SHA-256 stored, validation, listing, revocation and scope names. Every test runs on
//! SQLite and PostgreSQL.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;

use common::api_tokens::{Env, T0_MS, sha256_hex};
use deskhaven_core::api_tokens::{API_TOKEN_PREFIX, ApiTokenError, NewApiToken, Scope};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use deskhaven_db::time;

#[tokio::test]
async fn create_returns_a_dhk_token_once_and_stores_only_its_hash() {
    each_migrated_backend(async |db: &TestDb| {
        // Arrange
        let env = Env::new(db).await;
        let admin = env.admin("root").await;

        // Act
        let (token, secret) = env
            .tokens
            .create(env.new_token(Some(admin), &[Scope::Deploy, Scope::Assign]))
            .await
            .unwrap();

        // Assert
        let raw = secret.expose();
        assert!(raw.starts_with(API_TOKEN_PREFIX), "{}", db.backend);
        assert_eq!(raw.len(), API_TOKEN_PREFIX.len() + 43, "{}", db.backend);
        let row = env.row(token.id).await;
        assert_eq!(row.token_hash, sha256_hex(raw), "{}", db.backend);
        assert!(raw.starts_with(&row.token_prefix), "{}", db.backend);
        assert!(row.token_prefix.len() < raw.len() / 2, "{}", db.backend);
        assert_eq!(row.scopes, r#"["deploy","assign"]"#, "{}", db.backend);
        assert_eq!(row.created_by, Some(admin), "{}", db.backend);
        assert_eq!(row.created_at, time::from_millis(T0_MS).unwrap());
        assert!(
            !format!("{secret:?} {token:?}").contains(raw),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn create_rejects_blank_names_empty_scopes_and_past_expiry() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let past = time::from_millis(T0_MS - 1).unwrap();
        let cases = [
            NewApiToken {
                name: "  ".to_owned(),
                ..env.new_token(None, &[Scope::Deploy])
            },
            NewApiToken {
                name: "x".repeat(200),
                ..env.new_token(None, &[Scope::Deploy])
            },
            env.new_token(None, &[]),
            NewApiToken {
                expires_at: Some(past),
                ..env.new_token(None, &[Scope::Deploy])
            },
        ];

        for new in cases {
            let result = env.tokens.create(new).await;
            assert!(
                matches!(result, Err(ApiTokenError::Invalid { .. })),
                "{}: {result:?}",
                db.backend
            );
        }
        assert!(
            env.tokens.list().await.unwrap().is_empty(),
            "{}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn create_for_an_unknown_creator_fails() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;

        let result = env
            .tokens
            .create(env.new_token(Some(4242), &[Scope::Deploy]))
            .await;

        assert!(
            matches!(result, Err(ApiTokenError::CreatorNotFound)),
            "{}: {result:?}",
            db.backend
        );
    })
    .await;
}

#[tokio::test]
async fn list_shows_every_token_without_its_secret() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (a, _) = env.issue(None, &[Scope::Deploy]).await;
        let (b, raw_b) = env
            .issue(None, &[Scope::AdminRead, Scope::AdminWrite])
            .await;
        env.tokens.revoke(a).await.unwrap();

        let listed = env.tokens.list().await.unwrap();

        let ids: Vec<i64> = listed.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![a, b], "{}", db.backend);
        assert!(listed[0].revoked_at.is_some(), "{}", db.backend);
        assert_eq!(
            listed[1].scopes,
            BTreeSet::from([Scope::AdminRead, Scope::AdminWrite]),
            "{}",
            db.backend
        );
        assert!(raw_b.starts_with(&listed[1].token_prefix), "{}", db.backend);
    })
    .await;
}

#[test]
fn scopes_have_stable_wire_names() {
    let names: Vec<&str> = [
        Scope::Deploy,
        Scope::Assign,
        Scope::AdminRead,
        Scope::AdminWrite,
    ]
    .iter()
    .map(|s| s.as_str())
    .collect();

    assert_eq!(names, ["deploy", "assign", "admin:read", "admin:write"]);
    for name in names {
        assert_eq!(Scope::parse(name).map(Scope::as_str), Some(name));
    }
    assert_eq!(Scope::parse("admin"), None);
}
