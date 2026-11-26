//! No redirects followed, so a provider cannot bounce us elsewhere; no proxy from the environment;
//! bounded time and size; names resolve through the filtering resolver. The client is built on
//! first use, so a host without roots fails OIDC requests, not startup.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use openidconnect::{AsyncHttpClient, HttpRequest, HttpResponse};
use tokio::sync::OnceCell;
use url::Url;

use super::resolver::{FilteringResolver, Lookup, system_lookup};
use super::url_policy::EndpointPolicy;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
pub const MAX_RESPONSE_BYTES: usize = 256 * 1024;

/// Never carries a request or response body.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("URL refused by the outbound policy: {0}")]
    Refused(&'static str),
    #[error("response body larger than {MAX_RESPONSE_BYTES} bytes")]
    TooLarge,
    #[error("TLS client setup failed: {0}")]
    Tls(String),
    #[error("request failed")]
    Transport(#[source] reqwest::Error),
    #[error("malformed request or response: {0}")]
    Malformed(String),
}

pub type FetchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<HttpResponse, FetchError>> + Send + 'a>>;

/// Implementations enforce the transport limits; the caller applies the URL policy.
pub trait HttpFetch: Send + Sync {
    fn fetch(&self, request: HttpRequest) -> FetchFuture<'_>;
}

pub struct ReqwestFetch {
    client: OnceCell<reqwest::Client>,
    connect_timeout: Duration,
    request_timeout: Duration,
    lookup: Lookup,
}

impl Default for ReqwestFetch {
    fn default() -> Self {
        Self::with_timeouts(CONNECT_TIMEOUT, REQUEST_TIMEOUT)
    }
}

impl ReqwestFetch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeouts(connect_timeout: Duration, request_timeout: Duration) -> Self {
        Self {
            client: OnceCell::new(),
            connect_timeout,
            request_timeout,
            lookup: system_lookup(),
        }
    }

    pub fn with_lookup(self, lookup: Lookup) -> Self {
        Self { lookup, ..self }
    }

    async fn client(&self) -> Result<&reqwest::Client, FetchError> {
        self.client
            .get_or_try_init(|| async {
                let resolver = FilteringResolver::new(Arc::clone(&self.lookup));
                build_client(self.connect_timeout, self.request_timeout, resolver)
            })
            .await
    }
}

impl HttpFetch for ReqwestFetch {
    fn fetch(&self, request: HttpRequest) -> FetchFuture<'_> {
        Box::pin(async move {
            let client = self.client().await?;
            send(client, request).await
        })
    }
}

fn build_client(
    connect: Duration,
    request: Duration,
    resolver: FilteringResolver,
) -> Result<reqwest::Client, FetchError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = rustls_platform_verifier::Verifier::new(Arc::clone(&provider))
        .map_err(|err| FetchError::Tls(err.to_string()))?;
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|err| FetchError::Tls(err.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .dns_resolver(resolver)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .referer(false)
        .connect_timeout(connect)
        .timeout(request)
        .build()
        .map_err(FetchError::Transport)
}

async fn send(client: &reqwest::Client, request: HttpRequest) -> Result<HttpResponse, FetchError> {
    let (parts, body) = request.into_parts();
    let mut response = client
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .map_err(FetchError::Transport)?;
    let limit = u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX);
    if response.content_length().is_some_and(|len| len > limit) {
        return Err(FetchError::TooLarge);
    }
    let status = response.status();
    let headers = response.headers().clone();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(FetchError::Transport)? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(FetchError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut out = openidconnect::http::Response::new(bytes);
    *out.status_mut() = status;
    *out.headers_mut() = headers;
    Ok(out)
}

pub(crate) struct ProviderHttp<'a> {
    fetch: &'a dyn HttpFetch,
    policy: EndpointPolicy,
}

impl<'a> ProviderHttp<'a> {
    pub(crate) fn new(fetch: &'a dyn HttpFetch, policy: EndpointPolicy) -> Self {
        Self { fetch, policy }
    }
}

impl<'c> AsyncHttpClient<'c> for ProviderHttp<'_> {
    type Error = FetchError;
    type Future = FetchFuture<'c>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let url = Url::parse(&request.uri().to_string())
                .map_err(|_| FetchError::Malformed("request URL".to_owned()))?;
            self.policy.check(&url).map_err(FetchError::Refused)?;
            self.fetch.fetch(request).await
        })
    }
}
