//! OIDC protocol against a mock IdP (wiremock): the authorization request, PKCE, single-use
//! state, ID token verification (nonce, issuer, audience, expiry, signature), IdP errors,
//! flow expiry and key rotation. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use chrono::Duration as ChronoDuration;
use common::oidc::{Env, PUBLIC_URL, device};
use common::oidc_idp::{AUTH_CODE, AuthRequest, CLIENT_ID, KEY_B, TokenSpec, s256};
use deskhaven_core::oidc::{CallbackError, CallbackParams, FLOW_TTL, Failure, FlowKind, OidcError};
use deskhaven_db::entity::oidc_flow;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::EntityTrait;
use wiremock::ResponseTemplate;

fn failure(report: &deskhaven_core::oidc::CallbackReport) -> Option<Failure> {
    report.outcome.as_ref().err().copied()
}

#[tokio::test]
async fn authorization_url_carries_code_flow_scopes_state_nonce_and_s256_challenge() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;

        let (start, request) = env.begin().await;

        let p = &request.params;
        assert!(
            start
                .url
                .as_str()
                .starts_with(&format!("{}/protocol/openid-connect/auth?", env.idp.issuer)),
            "{}",
            db.backend
        );
        assert_eq!(p["response_type"], "code");
        assert_eq!(p["client_id"], CLIENT_ID);
        assert_eq!(p["redirect_uri"], format!("{PUBLIC_URL}/api/oidc/callback"));
        assert_eq!(p["code_challenge_method"], "S256");
        let scopes: Vec<&str> = p["scope"].split(' ').collect();
        assert!(
            scopes.contains(&"openid") && scopes.contains(&"email"),
            "{scopes:?}"
        );
        assert_eq!(request.state.len(), 43);
        assert_eq!(request.nonce.len(), 43);
        assert_ne!(request.state, request.nonce);
        assert_eq!(start.code.expose().len(), 43);
        // Only hashes and the sealed verifier are stored.
        let row = oidc_flow::Entity::find()
            .one(&db.conn)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(row.state_hash, request.state);
        assert_ne!(row.client_code_hash.as_deref(), Some(start.code.expose()));
        assert!(row.pkce_verifier_enc.starts_with("v1:"));
        assert_eq!(row.nonce, request.nonce);
    })
    .await;
}

#[tokio::test]
async fn the_code_is_exchanged_with_the_pkce_verifier_of_the_challenge() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, request) = env.begin().await;
        env.idp.mount_token(&env.spec("sub-1", &request)).await;

        env.callback(&request).await;

        let exchanges = env.idp.token_requests().await;
        assert_eq!(exchanges.len(), 1, "{}", db.backend);
        let form = &exchanges[0];
        assert_eq!(form["grant_type"], "authorization_code");
        assert_eq!(form["code"], AUTH_CODE);
        assert_eq!(
            form["redirect_uri"],
            format!("{PUBLIC_URL}/api/oidc/callback")
        );
        let verifier = &form["code_verifier"];
        assert!((43..=128).contains(&verifier.len()));
        assert_eq!(s256(verifier), request.code_challenge);
        // Client authentication (client_secret_basic), not in the form.
        assert!(!form.contains_key("client_secret"));
    })
    .await;
}

#[tokio::test]
async fn a_state_completes_once_and_a_replay_does_nothing() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (_, request) = env.begin().await;
        env.idp.mount_token(&env.spec("sub-1", &request)).await;

        let first = env.callback(&request).await;
        let params = CallbackParams {
            state: Some(&request.state),
            code: Some(AUTH_CODE),
            error: None,
        };
        let replay = env.oidc.complete_callback(&params).await;

        assert!(first.outcome.is_ok(), "{}: {first:?}", db.backend);
        assert_eq!(
            first.flow,
            FlowKind::Client {
                rustdesk_id: device().rustdesk_id
            }
        );
        assert!(
            matches!(replay, Err(CallbackError::InvalidState)),
            "{replay:?}"
        );
        assert_eq!(env.idp.token_requests().await.len(), 1);
    })
    .await;
}

#[tokio::test]
async fn unknown_missing_or_malformed_states_are_refused_without_a_request() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.begin().await;
        let unknown = "A".repeat(43);

        for state in [None, Some(""), Some("short"), Some(unknown.as_str())] {
            let params = CallbackParams {
                state,
                code: Some(AUTH_CODE),
                error: None,
            };
            let result = env.oidc.complete_callback(&params).await;
            assert!(
                matches!(result, Err(CallbackError::InvalidState)),
                "{}: {state:?}",
                db.backend
            );
        }
        assert_eq!(env.idp.token_requests().await.len(), 0);
    })
    .await;
}

#[tokio::test]
async fn id_tokens_failing_verification_are_rejected() {
    type Shape = fn(TokenSpec) -> TokenSpec;
    let cases: [(&str, Shape); 5] = [
        ("nonce mismatch", |s| TokenSpec {
            nonce: "another-nonce".to_owned(),
            ..s
        }),
        ("wrong issuer", |s| TokenSpec {
            issuer: Some("https://evil.example.com".to_owned()),
            ..s
        }),
        ("wrong audience", |s| TokenSpec {
            audience: Some("someone-else".to_owned()),
            ..s
        }),
        ("expired", |s| TokenSpec {
            expires_in: ChronoDuration::seconds(-5),
            ..s
        }),
        ("signed by a key not in the JWKS", |s| TokenSpec {
            key: KEY_B,
            ..s
        }),
    ];
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        for (case, shape) in cases {
            let (_, report) = env.login("sub-1", shape).await;
            assert_eq!(
                failure(&report),
                Some(Failure::InvalidToken),
                "{}: {case}",
                db.backend
            );
            assert_eq!(report.subject, None, "{case}");
        }
        let flows = oidc_flow::Entity::find().all(&db.conn).await.unwrap();
        assert!(
            flows
                .iter()
                .all(|f| f.error.as_deref() == Some("oidc_invalid_token")
                    && f.completed_user_id.is_none())
        );
    })
    .await;
}

#[tokio::test]
async fn a_token_response_without_id_token_or_an_error_response_fails_the_flow() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;

        let (_, request) = env.begin().await;
        env.idp.mount_token(&env.spec("x", &request)).await;
        env.idp.server.reset().await;
        env.idp
            .mount_token_response(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "a", "token_type": "Bearer"
            })))
            .await;
        let no_id_token = env.callback(&request).await;

        let (_, request) = env.begin().await;
        env.idp.server.reset().await;
        env.idp
            .mount_token_response(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": "invalid_grant"
            })))
            .await;
        let rejected = env.callback(&request).await;

        assert_eq!(
            failure(&no_id_token),
            Some(Failure::InvalidToken),
            "{}",
            db.backend
        );
        assert_eq!(failure(&rejected), Some(Failure::Unavailable));
    })
    .await;
}

#[tokio::test]
async fn an_idp_error_or_a_missing_code_is_a_denied_flow() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let (_, denied) = env.begin().await;
        let (_, no_code) = env.begin().await;

        let denied = env
            .oidc
            .complete_callback(&CallbackParams {
                state: Some(&denied.state),
                code: None,
                error: Some("access_denied"),
            })
            .await
            .unwrap();
        let no_code = env
            .oidc
            .complete_callback(&CallbackParams {
                state: Some(&no_code.state),
                code: None,
                error: None,
            })
            .await
            .unwrap();

        assert_eq!(failure(&denied), Some(Failure::Denied), "{}", db.backend);
        assert_eq!(failure(&no_code), Some(Failure::Denied));
        assert_eq!(env.idp.token_requests().await.len(), 0);
    })
    .await;
}

#[tokio::test]
async fn an_expired_flow_cannot_be_completed_and_is_cleaned_up() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        let (_, request) = env.begin().await;
        env.idp.mount_token(&env.spec("sub-1", &request)).await;
        env.clock.advance(FLOW_TTL);

        let params = CallbackParams {
            state: Some(&request.state),
            code: Some(AUTH_CODE),
            error: None,
        };
        let late = env.oidc.complete_callback(&params).await;
        let removed = env.oidc.cleanup_flows().await.unwrap();

        assert!(
            matches!(late, Err(CallbackError::InvalidState)),
            "{}",
            db.backend
        );
        assert_eq!(removed, 1);
        assert_eq!(
            oidc_flow::Entity::find().all(&db.conn).await.unwrap().len(),
            0
        );
    })
    .await;
}

#[tokio::test]
async fn a_rotated_signing_key_is_picked_up_by_one_jwks_refetch() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        // Warm the cache with KEY_A's JWKS.
        env.login("sub-1", |s| s).await;
        // The IdP rotates to KEY_B under a new kid.
        let (_, request) = env.begin().await;
        let spec = TokenSpec {
            key: KEY_B,
            kid: "test-key-2",
            ..env.spec("sub-1", &request)
        };
        rotate_to(&env, &spec).await;
        env.clock.advance(std::time::Duration::from_secs(61));

        let report = env.callback(&request).await;

        assert!(report.outcome.is_ok(), "{}: {report:?}", db.backend);
        assert_eq!(env.idp.hits("/certs").await, 1);
    })
    .await;
}

#[tokio::test]
async fn unknown_key_ids_refetch_at_most_once_per_minute() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.auto_provision = true).await;
        env.login("sub-1", |s| s).await;
        let forged = |env: &Env, request: &AuthRequest| TokenSpec {
            key: KEY_B,
            kid: "forged",
            ..env.spec("sub-1", request)
        };

        // Younger than a minute: no refetch at all.
        let (_, request) = env.begin().await;
        rotate_to(&env, &forged(&env, &request)).await;
        let young = env.callback(&request).await;
        assert_eq!(env.idp.hits("/certs").await, 0);
        // Older: one refetch, still no such key.
        env.clock.advance(std::time::Duration::from_secs(61));
        let (_, request) = env.begin().await;
        rotate_to(&env, &forged(&env, &request)).await;
        let old = env.callback(&request).await;

        assert_eq!(
            failure(&young),
            Some(Failure::InvalidToken),
            "{}",
            db.backend
        );
        assert_eq!(failure(&old), Some(Failure::InvalidToken));
        assert_eq!(env.idp.hits("/certs").await, 1);
    })
    .await;
}

/// The mock now publishes only KEY_B (kid `test-key-2`) and answers with `spec`'s token.
async fn rotate_to(env: &Env, spec: &TokenSpec) {
    env.idp.server.reset().await;
    env.idp.mount_discovery(env.idp.discovery()).await;
    env.idp.mount_jwks(KEY_B, "test-key-2").await;
    let body = serde_json::json!({
        "access_token": "a", "token_type": "Bearer", "id_token": env.idp.id_token(spec)
    });
    env.idp
        .mount_token_response(ResponseTemplate::new(200).set_body_json(body))
        .await;
}

#[tokio::test]
async fn begin_refuses_unknown_disabled_or_unconfigured_providers_and_bad_devices() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |p| p.enabled = true).await;
        let mut bad_device = device();
        bad_device.uuid = String::new();

        let unknown = env.oidc.begin_client("nope", &device()).await;
        let bad = env.oidc.begin_client("corp", &bad_device).await;
        env.oidc
            .update_provider(
                env.provider.id,
                deskhaven_core::oidc::ProviderUpdate {
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let disabled = env.oidc.begin_client("corp", &device()).await;

        assert!(
            matches!(unknown, Err(OidcError::UnknownProvider)),
            "{}",
            db.backend
        );
        assert!(matches!(bad, Err(OidcError::InvalidDevice)));
        assert!(matches!(disabled, Err(OidcError::UnknownProvider)));
    })
    .await;
}

#[tokio::test]
async fn an_unreachable_or_lying_discovery_document_makes_the_provider_unavailable() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        env.idp.server.reset().await;
        let mut lying = env.idp.discovery();
        lying["issuer"] = serde_json::json!("https://other.example.com");
        env.idp.mount_discovery(lying).await;

        let result = env.oidc.begin_client("corp", &device()).await;

        assert!(
            matches!(result, Err(OidcError::Unavailable(_))),
            "{}: {result:?}",
            db.backend
        );
        assert_eq!(
            oidc_flow::Entity::find().all(&db.conn).await.unwrap().len(),
            0
        );
    })
    .await;
}
