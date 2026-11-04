//! The seam between the LDAP service and a directory server: [`Connector`] opens a
//! [`Session`], which binds and searches. Production: [`Ldap3Connector`](super::Ldap3Connector);
//! tests plug an in-memory directory in.
//!
//! [`DirectoryFault`] never carries a password: implementations describe transport and
//! protocol failures only (the server's diagnostic text included, which names DNs at most).

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

/// Where and how to connect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// `ldap://host[:port]` or `ldaps://host[:port]`, as validated by the directory service.
    pub url: String,
    /// Upgrade an `ldap://` connection with StartTLS before anything is sent.
    pub start_tls: bool,
    /// Verify the server certificate (chain and host name). Off only when set explicitly.
    pub tls_verify: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The base entry only.
    Base,
    /// The base entry and everything below it.
    Subtree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRequest {
    pub base: String,
    pub scope: Scope,
    /// An RFC 4515 filter; every value taken from input is already escaped.
    pub filter: String,
    /// Attributes to return (none: only the DN).
    pub attrs: Vec<String>,
    /// Most entries wanted; more matches set [`Found::truncated`].
    pub size_limit: usize,
}

/// A search result entry. Attribute names are ASCII-lowercased; values that are not UTF-8
/// are left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub dn: String,
    pub attrs: BTreeMap<String, Vec<String>>,
}

impl Entry {
    /// The first value of `attr` (any case).
    pub fn first(&self, attr: &str) -> Option<&str> {
        self.values(attr).first().map(String::as_str)
    }

    /// Every value of `attr` (any case).
    pub fn values(&self, attr: &str) -> &[String] {
        self.attrs
            .get(&attr.to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub entries: Vec<Entry>,
    /// More entries matched than [`SearchRequest::size_limit`] (or the server's own limit).
    pub truncated: bool,
}

/// Why a directory operation failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DirectoryFault {
    /// A bind was refused (result code 49): wrong DN or password.
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("cannot reach the directory: {0}")]
    Unreachable(String),
    #[error("TLS failed: {0}")]
    Tls(String),
    #[error("the directory did not answer in time")]
    Timeout,
    /// Any other LDAP result code.
    #[error("the directory refused the operation (result code {code}): {message}")]
    Refused { code: u32, message: String },
}

pub type DirectoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, DirectoryFault>> + Send + 'a>>;

/// Opens directory sessions.
pub trait Connector: Send + Sync {
    /// Connects (with TLS as `target` says). The session starts anonymous.
    fn connect<'a>(&'a self, target: &'a Target) -> DirectoryFuture<'a, Box<dyn Session>>;
}

/// One connection to a directory.
pub trait Session: Send {
    /// A simple bind. Callers never pass an empty password (RFC 4513 §5.1.2: that is an
    /// unauthenticated bind, which many servers accept).
    fn bind<'a>(&'a mut self, dn: &'a str, password: &'a str) -> DirectoryFuture<'a, ()>;

    fn search<'a>(&'a mut self, request: &'a SearchRequest) -> DirectoryFuture<'a, Found>;

    /// Unbinds and closes, best effort.
    fn close(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}
