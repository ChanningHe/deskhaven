//! A mock OpenID provider on wiremock: discovery document, JWKS and a token endpoint that
//! answers with an ID token the test describes ([`TokenSpec`]), signed RS256 with a
//! throwaway fixture key (`tests/fixtures/oidc/idp-key-*.pem`, PKCS#1, generated for these
//! tests only). Shared by the core and contract tests (`#[path]` include).
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic, clippy::expect_used)]
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use openidconnect::core::{
    CoreGenderClaim, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm,
    CoreRsaPrivateSigningKey,
};
use openidconnect::{
    AdditionalClaims, Audience, EndUserEmail, EndUserName, EndUserUsername, IdToken, IdTokenClaims,
    IssuerUrl, JsonWebKeyId, LocalizedClaim, Nonce, PrivateSigningKey, StandardClaims,
    SubjectIdentifier,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub const KEY_A: &str = include_str!("../fixtures/oidc/idp-key-a.pem");
pub const KEY_B: &str = include_str!("../fixtures/oidc/idp-key-b.pem");
pub const KID: &str = "test-key-1";
pub const CLIENT_ID: &str = "deskhaven-test";
pub const CLIENT_SECRET: &str = "test-client-secret";
pub const ISSUER_PATH: &str = "/realms/test";
pub const AUTH_CODE: &str = "auth-code-from-the-idp";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Extra(pub Map<String, Value>);

impl AdditionalClaims for Extra {}

pub struct MockIdp {
    pub server: MockServer,
    /// `<server>/realms/test`, as configured on the provider.
    pub issuer: String,
}

/// What the next ID token says. `Default` + [`TokenSpec::for_flow`] is a valid token.
#[derive(Clone, Debug)]
pub struct TokenSpec {
    pub subject: String,
    pub nonce: String,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
    pub preferred_username: Option<String>,
    pub name: Option<String>,
    pub extra: Map<String, Value>,
    /// Overrides the issuer (default: the mock's).
    pub issuer: Option<String>,
    /// Overrides the audience (default: [`CLIENT_ID`]).
    pub audience: Option<String>,
    /// Further audiences next to it.
    pub extra_audiences: Vec<String>,
    /// Expiry relative to `now`.
    pub expires_in: ChronoDuration,
    /// The test's notion of now (the service clock).
    pub now: DateTime<Utc>,
    /// Signing key PEM (default [`KEY_A`], the one published in the JWKS).
    pub key: &'static str,
    /// `kid` of the token header (default [`KID`]).
    pub kid: &'static str,
}

impl TokenSpec {
    pub fn for_flow(subject: &str, nonce: &str, now_ms: i64) -> Self {
        Self {
            subject: subject.to_owned(),
            nonce: nonce.to_owned(),
            email: None,
            email_verified: None,
            preferred_username: None,
            name: None,
            extra: Map::new(),
            issuer: None,
            audience: None,
            extra_audiences: Vec::new(),
            expires_in: ChronoDuration::minutes(5),
            now: DateTime::from_timestamp_millis(now_ms).unwrap(),
            key: KEY_A,
            kid: KID,
        }
    }

    pub fn email(mut self, email: &str, verified: Option<bool>) -> Self {
        self.email = Some(email.to_owned());
        self.email_verified = verified;
        self
    }
}

/// `state`, `nonce` and the PKCE challenge of an authorization URL.
#[derive(Debug)]
pub struct AuthRequest {
    pub state: String,
    pub nonce: String,
    pub code_challenge: String,
    pub params: HashMap<String, String>,
}

impl AuthRequest {
    pub fn parse(url: &Url) -> Self {
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();
        Self {
            state: params["state"].clone(),
            nonce: params["nonce"].clone(),
            code_challenge: params["code_challenge"].clone(),
            params,
        }
    }
}

/// base64url(SHA-256(verifier)), the S256 challenge.
pub fn s256(verifier: &str) -> String {
    use base64_022::Engine as _;
    base64_022::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

impl MockIdp {
    /// Discovery and JWKS mounted; no token endpoint yet.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        let issuer = format!("{}{ISSUER_PATH}", server.uri());
        let idp = Self { server, issuer };
        idp.mount_discovery(idp.discovery()).await;
        idp.mount_jwks(KEY_A, KID).await;
        idp
    }

    pub fn discovery(&self) -> Value {
        json!({
            "issuer": self.issuer,
            "authorization_endpoint": format!("{}/protocol/openid-connect/auth", self.issuer),
            "token_endpoint": format!("{}/protocol/openid-connect/token", self.issuer),
            "jwks_uri": format!("{}/protocol/openid-connect/certs", self.issuer),
            "response_types_supported": ["code"],
            "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["RS256"],
            "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        })
    }

    pub async fn mount_discovery(&self, document: Value) {
        Mock::given(method("GET"))
            .and(path(format!(
                "{ISSUER_PATH}/.well-known/openid-configuration"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(document))
            .mount(&self.server)
            .await;
    }

    pub async fn mount_jwks(&self, pem: &str, kid: &str) {
        let key = signing_key(pem, kid);
        let jwks = json!({ "keys": [serde_json::to_value(key.as_verification_key()).unwrap()] });
        Mock::given(method("GET"))
            .and(path(format!("{ISSUER_PATH}/protocol/openid-connect/certs")))
            .respond_with(ResponseTemplate::new(200).set_body_json(jwks))
            .mount(&self.server)
            .await;
    }

    /// Resets the mock (mocks and recorded requests), then discovery, the JWKS of
    /// [`KEY_A`] and a token endpoint answering every exchange with an ID token per `spec`.
    pub async fn mount_token(&self, spec: &TokenSpec) {
        self.server.reset().await;
        self.mount_discovery(self.discovery()).await;
        self.mount_jwks(KEY_A, KID).await;
        let body = json!({
            "access_token": "mock-access-token",
            "token_type": "Bearer",
            "expires_in": 300,
            "id_token": self.id_token(spec),
        });
        self.mount_token_response(ResponseTemplate::new(200).set_body_json(body))
            .await;
    }

    pub async fn mount_token_response(&self, response: ResponseTemplate) {
        Mock::given(method("POST"))
            .and(path(format!("{ISSUER_PATH}/protocol/openid-connect/token")))
            .respond_with(response)
            .mount(&self.server)
            .await;
    }

    pub fn id_token(&self, spec: &TokenSpec) -> String {
        let issuer = spec.issuer.clone().unwrap_or_else(|| self.issuer.clone());
        let audience = spec
            .audience
            .clone()
            .unwrap_or_else(|| CLIENT_ID.to_owned());
        let mut standard = StandardClaims::new(SubjectIdentifier::new(spec.subject.clone()))
            .set_email(spec.email.clone().map(EndUserEmail::new))
            .set_email_verified(spec.email_verified)
            .set_preferred_username(spec.preferred_username.clone().map(EndUserUsername::new));
        if let Some(name) = &spec.name {
            standard =
                standard.set_name(Some(LocalizedClaim::from(EndUserName::new(name.clone()))));
        }
        let audiences = std::iter::once(audience)
            .chain(spec.extra_audiences.iter().cloned())
            .map(Audience::new)
            .collect();
        let claims: IdTokenClaims<Extra, CoreGenderClaim> = IdTokenClaims::new(
            IssuerUrl::new(issuer).unwrap(),
            audiences,
            spec.now + spec.expires_in,
            spec.now,
            standard,
            Extra(spec.extra.clone()),
        )
        .set_nonce(Some(Nonce::new(spec.nonce.clone())));
        let token = IdToken::<
            Extra,
            CoreGenderClaim,
            CoreJweContentEncryptionAlgorithm,
            CoreJwsSigningAlgorithm,
        >::new(
            claims,
            &signing_key(spec.key, spec.kid),
            CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
            None,
            None,
        )
        .unwrap();
        token.to_string()
    }

    /// A JWT with the claims of `spec`, header `{"alg": alg, "kid": KID}` and the signature
    /// `sign(signing input)` (algorithm-confusion tests: HS256, `none`).
    pub fn raw_token(
        &self,
        spec: &TokenSpec,
        alg: &str,
        sign: impl Fn(&[u8]) -> Vec<u8>,
    ) -> String {
        use base64_022::Engine as _;
        let b64 = |bytes: &[u8]| base64_022::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let header = json!({ "alg": alg, "typ": "JWT", "kid": KID });
        let claims = json!({
            "iss": spec.issuer.clone().unwrap_or_else(|| self.issuer.clone()),
            "aud": spec.audience.clone().unwrap_or_else(|| CLIENT_ID.to_owned()),
            "sub": spec.subject,
            "exp": (spec.now + spec.expires_in).timestamp(),
            "iat": spec.now.timestamp(),
            "nonce": spec.nonce,
        });
        let input = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let signature = sign(input.as_bytes());
        format!("{input}.{}", b64(&signature))
    }

    /// The token endpoint answers with this raw ID token (discovery and JWKS remounted).
    pub async fn mount_raw_token(&self, id_token: &str, discovery: Value) {
        self.server.reset().await;
        self.mount_discovery(discovery).await;
        self.mount_jwks(KEY_A, KID).await;
        let body = json!({
            "access_token": "mock-access-token", "token_type": "Bearer", "id_token": id_token,
        });
        self.mount_token_response(ResponseTemplate::new(200).set_body_json(body))
            .await;
    }

    /// Form bodies of every token request received so far.
    pub async fn token_requests(&self) -> Vec<HashMap<String, String>> {
        let requests = self.server.received_requests().await.unwrap_or_default();
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("/token"))
            .map(|r| url::form_urlencoded::parse(&r.body).into_owned().collect())
            .collect()
    }

    /// Number of requests received for paths ending in `suffix`.
    pub async fn hits(&self, suffix: &str) -> usize {
        let requests = self.server.received_requests().await.unwrap_or_default();
        requests
            .iter()
            .filter(|r| r.url.path().ends_with(suffix))
            .count()
    }
}

fn signing_key(pem: &str, kid: &str) -> CoreRsaPrivateSigningKey {
    CoreRsaPrivateSigningKey::from_pem(pem, Some(JsonWebKeyId::new(kid.to_owned()))).unwrap()
}
