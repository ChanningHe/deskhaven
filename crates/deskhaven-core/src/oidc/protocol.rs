//! The relying-party protocol over `openidconnect`: the authorization URL, the code exchange
//! with the PKCE verifier, and ID token verification.
//!
//! Verification: signature with a key of the provider's JWKS under an asymmetric algorithm
//! only ([`ALLOWED_ALGS`]: no HMAC with the client secret, never `none`), `iss` equal to
//! the configured issuer, `aud` containing the client id and no untrusted extra audience,
//! `exp` against the service clock, and `nonce` equal to the flow's. The access token is not
//! used (no userinfo call), so `at_hash` is not checked.

use openidconnect::core::{
    CoreAuthDisplay, CoreAuthPrompt, CoreAuthenticationFlow, CoreClientAuthMethod,
    CoreErrorResponseType, CoreGenderClaim, CoreJsonWebKey, CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm, CoreRevocableToken, CoreRevocationErrorResponse,
    CoreTokenIntrospectionResponse, CoreTokenType,
};
use openidconnect::{
    AuthType, AuthorizationCode, ClaimsVerificationError, Client, ClientId, ClientSecret,
    CsrfToken, EmptyExtraTokenFields, EndpointMaybeSet, EndpointNotSet, EndpointSet, IdToken,
    IdTokenFields, Nonce, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RequestTokenError,
    Scope, SignatureVerificationError, StandardErrorResponse, StandardTokenResponse,
    TokenResponse as _,
};
use url::Url;

use super::claims::{Claims, ExtraClaims};
use super::discovery::{Discovered, fetch_kind};
use super::http::{FetchError, HttpFetch, ProviderHttp};
use crate::clock::SharedClock;

/// Signature algorithms accepted for ID tokens.
const ALLOWED_ALGS: [CoreJwsSigningAlgorithm; 9] = [
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha384,
    CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha512,
    CoreJwsSigningAlgorithm::RsaSsaPssSha256,
    CoreJwsSigningAlgorithm::RsaSsaPssSha384,
    CoreJwsSigningAlgorithm::RsaSsaPssSha512,
    CoreJwsSigningAlgorithm::EcdsaP256Sha256,
    CoreJwsSigningAlgorithm::EcdsaP384Sha384,
    CoreJwsSigningAlgorithm::EdDsa,
];

pub(crate) type OidcIdToken = IdToken<
    ExtraClaims,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;
type TokenFields = IdTokenFields<
    ExtraClaims,
    EmptyExtraTokenFields,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJwsSigningAlgorithm,
>;
type TokenResponse = StandardTokenResponse<TokenFields, CoreTokenType>;
type RpClient = Client<
    ExtraClaims,
    CoreAuthDisplay,
    CoreGenderClaim,
    CoreJweContentEncryptionAlgorithm,
    CoreJsonWebKey,
    CoreAuthPrompt,
    StandardErrorResponse<CoreErrorResponseType>,
    TokenResponse,
    CoreTokenIntrospectionResponse,
    CoreRevocableToken,
    CoreRevocationErrorResponse,
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

/// What identifies the relying party at the provider.
pub(crate) struct RelyingParty<'a> {
    pub(crate) client_id: &'a str,
    pub(crate) client_secret: Option<&'a str>,
    pub(crate) redirect_uri: &'a Url,
}

/// The per-flow secrets.
pub(crate) struct FlowSecrets<'a> {
    pub(crate) state: &'a str,
    pub(crate) nonce: &'a str,
    pub(crate) pkce_verifier: &'a str,
}

/// Why the exchange or the verification failed: `Unavailable` is the provider's (network,
/// error response), `Invalid` the token's.
#[derive(Debug)]
pub(crate) enum ExchangeError {
    Unavailable(String),
    Invalid(String),
}

fn client(discovered: &Discovered, rp: &RelyingParty<'_>) -> RpClient {
    let metadata = discovered.metadata.clone();
    let basic_supported = metadata
        .token_endpoint_auth_methods_supported()
        .is_none_or(|methods| methods.contains(&CoreClientAuthMethod::ClientSecretBasic));
    let client = RpClient::from_provider_metadata(
        metadata,
        ClientId::new(rp.client_id.to_owned()),
        rp.client_secret.map(|s| ClientSecret::new(s.to_owned())),
    )
    .set_redirect_uri(RedirectUrl::from_url(rp.redirect_uri.clone()));
    if basic_supported {
        client
    } else {
        client.set_auth_type(AuthType::RequestBody)
    }
}

/// The URL the user's browser opens: code flow, the scopes, `state`, `nonce` and the S256
/// challenge of the verifier.
pub(crate) fn authorization_url(
    discovered: &Discovered,
    rp: &RelyingParty<'_>,
    scopes: &str,
    secrets: &FlowSecrets<'_>,
) -> Url {
    let (state, nonce) = (secrets.state.to_owned(), secrets.nonce.to_owned());
    let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
        secrets.pkce_verifier.to_owned(),
    ));
    let extra_scopes = scopes
        .split_whitespace()
        .filter(|s| *s != "openid")
        .map(|s| Scope::new(s.to_owned()));
    let (url, _, _) = client(discovered, rp)
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            move || CsrfToken::new(state),
            move || Nonce::new(nonce),
        )
        .add_scopes(extra_scopes)
        .set_pkce_challenge(challenge)
        .url();
    url
}

/// The ID token the token endpoint returns for `code`, sent with the PKCE verifier.
pub(crate) async fn exchange(
    discovered: &Discovered,
    fetch: &dyn HttpFetch,
    rp: &RelyingParty<'_>,
    code: &str,
    pkce_verifier: &str,
) -> Result<OidcIdToken, ExchangeError> {
    let client = client(discovered, rp);
    let http = ProviderHttp::new(fetch, discovered.policy);
    let request = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .map_err(|err| ExchangeError::Unavailable(err.to_string()))?
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()));
    let response = request.request_async(&http).await.map_err(|err| {
        ExchangeError::Unavailable(format!("token request: {}", token_kind(&err)))
    })?;
    response
        .id_token()
        .cloned()
        .ok_or_else(|| ExchangeError::Invalid("the token response has no ID token".to_owned()))
}

/// The verified claims of `token`.
pub(crate) fn verify(
    discovered: &Discovered,
    rp: &RelyingParty<'_>,
    clock: &SharedClock,
    token: &OidcIdToken,
    nonce: &str,
) -> Result<Claims, ClaimsVerificationError> {
    let client = client(discovered, rp);
    let clock = clock.clone();
    let verifier = client
        .id_token_verifier()
        .set_allowed_algs(ALLOWED_ALGS)
        .set_time_fn(move || *clock.now());
    token
        .claims(&verifier, &Nonce::new(nonce.to_owned()))
        .cloned()
}

type TokenError = RequestTokenError<FetchError, StandardErrorResponse<CoreErrorResponseType>>;

/// What went wrong with the token request, without the response body (a parse error's
/// message can quote token values): the OAuth error code at most.
fn token_kind(err: &TokenError) -> String {
    match err {
        RequestTokenError::ServerResponse(response) => {
            format!("error response {}", response.error())
        }
        RequestTokenError::Request(fetch) => fetch_kind(fetch),
        RequestTokenError::Parse(_, _) => "unparsable token response".to_owned(),
        RequestTokenError::Other(_) => "unexpected token response".to_owned(),
    }
}

/// The kind of an ID token rejection, without claim values (for logs).
pub(crate) fn rejection_kind(err: &ClaimsVerificationError) -> &'static str {
    match err {
        ClaimsVerificationError::Expired(_) => "expired",
        ClaimsVerificationError::InvalidAudience(_) => "audience",
        ClaimsVerificationError::InvalidAuthContext(_) => "auth context",
        ClaimsVerificationError::InvalidAuthTime(_) => "auth time",
        ClaimsVerificationError::InvalidIssuer(_) => "issuer",
        ClaimsVerificationError::InvalidNonce(_) => "nonce",
        ClaimsVerificationError::InvalidSubject(_) => "subject",
        ClaimsVerificationError::SignatureVerification(_) => "signature",
        ClaimsVerificationError::Unsupported(_) => "unsupported",
        _ => "other",
    }
}

/// Whether the failure may be a signing key rotated since the JWKS was fetched.
pub(crate) fn is_unknown_key(err: &ClaimsVerificationError) -> bool {
    matches!(
        err,
        ClaimsVerificationError::SignatureVerification(SignatureVerificationError::NoMatchingKey)
    )
}
