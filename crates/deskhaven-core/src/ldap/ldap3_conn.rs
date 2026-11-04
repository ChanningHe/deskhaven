//! [`Connector`] over ldap3 0.12 (tokio, rustls + ring): one connection per login or
//! diagnostic, [`CONNECT_TIMEOUT`] for name resolution, TCP and TLS (StartTLS included),
//! [`OPERATION_TIMEOUT`] for every bind and search. The TCP connection goes to an address
//! `address` resolved and vetted (no link-local or metadata addresses); TLS still checks the
//! URL's host name. Referrals are not followed (they are dropped from results).

use std::io;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use ldap3::{LdapConnAsync, LdapConnSettings, LdapError, SearchEntry, SearchOptions, StdStream};
use rustls::ClientConfig;

use super::directory::{
    Connector, DirectoryFault, DirectoryFuture, Entry, Found, Scope, SearchRequest, Session, Target,
};
use super::{address, tls};
use crate::oidc::{Lookup, system_lookup};

/// TCP connect plus TLS handshake.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// One bind or search.
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
/// LDAP `invalidCredentials`.
const INVALID_CREDENTIALS: u32 = 49;
/// LDAP `sizeLimitExceeded`: the entries so far are valid, there are more.
const SIZE_LIMIT_EXCEEDED: u32 = 4;
/// Longest server diagnostic message kept in a fault.
const MAX_MESSAGE_CHARS: usize = 200;

pub struct Ldap3Connector {
    connect_timeout: Duration,
    operation_timeout: Duration,
    verified: OnceLock<Result<Arc<ClientConfig>, String>>,
    /// Resolves directory host names (production: the system resolver).
    lookup: Lookup,
}

impl Default for Ldap3Connector {
    fn default() -> Self {
        Self::with_timeouts(CONNECT_TIMEOUT, OPERATION_TIMEOUT)
    }
}

impl Ldap3Connector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeouts(connect_timeout: Duration, operation_timeout: Duration) -> Self {
        Self {
            connect_timeout,
            operation_timeout,
            verified: OnceLock::new(),
            lookup: system_lookup(),
        }
    }

    /// The same connector resolving host names with `lookup` (tests).
    pub fn with_lookup(self, lookup: Lookup) -> Self {
        Self { lookup, ..self }
    }

    /// The verifying config is built once (it loads the OS roots); a host without roots
    /// fails directory logins (logged), not startup.
    fn tls_config(&self, verify: bool) -> Result<Arc<ClientConfig>, DirectoryFault> {
        let config = if verify {
            self.verified.get_or_init(tls::verified_config).clone()
        } else {
            tls::unverified_config()
        };
        config.map_err(DirectoryFault::Tls)
    }
}

impl Ldap3Connector {
    /// TCP to a vetted address, then ldap3 (StartTLS / TLS with the URL's host name).
    async fn open(
        &self,
        target: &Target,
        config: Arc<ClientConfig>,
    ) -> Result<(LdapConnAsync, ldap3::Ldap), DirectoryFault> {
        let stream = address::connect(&self.lookup, &target.url).await?;
        let stream = stream
            .into_std()
            .map_err(|err| DirectoryFault::Unreachable(err.to_string()))?;
        let settings = LdapConnSettings::new()
            .set_std_stream(StdStream::Tcp(stream))
            .set_starttls(target.start_tls)
            .set_config(config)
            // Only lets ldap3 accept IP-literal hosts without verification; the
            // verifier itself is the config's.
            .set_no_tls_verify(!target.tls_verify);
        LdapConnAsync::with_settings(settings, &target.url)
            .await
            .map_err(fault)
    }
}

impl Connector for Ldap3Connector {
    fn connect<'a>(&'a self, target: &'a Target) -> DirectoryFuture<'a, Box<dyn Session>> {
        Box::pin(async move {
            let config = self.tls_config(target.tls_verify)?;
            let opened = tokio::time::timeout(self.connect_timeout, self.open(target, config));
            let (conn, ldap) = opened.await.map_err(|_| DirectoryFault::Timeout)??;
            tokio::spawn(async move {
                if let Err(err) = conn.drive().await {
                    tracing::debug!(error = %err, "LDAP connection ended with an error");
                }
            });
            let session: Box<dyn Session> = Box::new(Ldap3Session {
                ldap,
                timeout: self.operation_timeout,
            });
            Ok(session)
        })
    }
}

struct Ldap3Session {
    ldap: ldap3::Ldap,
    timeout: Duration,
}

impl Session for Ldap3Session {
    fn bind<'a>(&'a mut self, dn: &'a str, password: &'a str) -> DirectoryFuture<'a, ()> {
        Box::pin(async move {
            let result = self
                .ldap
                .with_timeout(self.timeout)
                .simple_bind(dn, password)
                .await
                .map_err(fault)?;
            result.success().map(|_| ()).map_err(fault)
        })
    }

    fn search<'a>(&'a mut self, request: &'a SearchRequest) -> DirectoryFuture<'a, Found> {
        Box::pin(async move {
            let scope = match request.scope {
                Scope::Base => ldap3::Scope::Base,
                Scope::Subtree => ldap3::Scope::Subtree,
            };
            let limit = i32::try_from(request.size_limit).unwrap_or(i32::MAX);
            let ldap3::SearchResult(entries, result) = self
                .ldap
                .with_timeout(self.timeout)
                .with_search_options(SearchOptions::new().sizelimit(limit))
                .search(&request.base, scope, &request.filter, request.attrs.clone())
                .await
                .map_err(fault)?;
            let mut truncated = match result.rc {
                0 => false,
                SIZE_LIMIT_EXCEEDED => true,
                _ => return Err(fault(LdapError::from(result))),
            };
            let mut entries: Vec<Entry> = entries
                .into_iter()
                .filter(|e| !e.is_ref() && !e.is_intermediate())
                .map(|e| entry(SearchEntry::construct(e)))
                .collect();
            if entries.len() > request.size_limit {
                entries.truncate(request.size_limit);
                truncated = true;
            }
            Ok(Found { entries, truncated })
        })
    }

    fn close(&mut self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            if let Err(err) = self.ldap.unbind().await {
                tracing::debug!(error = %err, "LDAP unbind failed");
            }
        })
    }
}

fn entry(found: SearchEntry) -> Entry {
    let attrs = found
        .attrs
        .into_iter()
        .map(|(name, values)| (name.to_ascii_lowercase(), values))
        .collect();
    Entry {
        dn: found.dn,
        attrs,
    }
}

fn fault(err: LdapError) -> DirectoryFault {
    match err {
        LdapError::LdapResult { result } if result.rc == INVALID_CREDENTIALS => {
            DirectoryFault::InvalidCredentials
        }
        LdapError::LdapResult { result } => DirectoryFault::Refused {
            code: result.rc,
            message: result.text.chars().take(MAX_MESSAGE_CHARS).collect(),
        },
        LdapError::Timeout { .. } => DirectoryFault::Timeout,
        LdapError::Rustls { source } => DirectoryFault::Tls(source.to_string()),
        LdapError::DNSName { source } => DirectoryFault::Tls(source.to_string()),
        LdapError::Io { source } => io_fault(&source),
        other => DirectoryFault::Unreachable(other.to_string()),
    }
}

/// TLS handshake failures arrive as I/O errors wrapping the rustls error.
fn io_fault(err: &io::Error) -> DirectoryFault {
    match err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(tls) => DirectoryFault::Tls(tls.to_string()),
        None if err.kind() == io::ErrorKind::TimedOut => DirectoryFault::Timeout,
        None => DirectoryFault::Unreachable(err.to_string()),
    }
}
