//! [`DirectoryFault`] never carries a password: transport and protocol failures only.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub url: String,
    pub start_tls: bool,
    /// Off only when set explicitly.
    pub tls_verify: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Base,
    Subtree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRequest {
    pub base: String,
    pub scope: Scope,
    /// Every value taken from input is already escaped.
    pub filter: String,
    /// An empty list asks for all user attributes; `["1.1"]` for none.
    pub attrs: Vec<String>,
    pub size_limit: usize,
}

/// Values that are not UTF-8 are left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub dn: String,
    pub attrs: BTreeMap<String, Vec<String>>,
}

impl Entry {
    pub fn first(&self, attr: &str) -> Option<&str> {
        self.values(attr).first().map(String::as_str)
    }

    pub fn values(&self, attr: &str) -> &[String] {
        self.attrs
            .get(&attr.to_ascii_lowercase())
            .map_or(&[], Vec::as_slice)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub entries: Vec<Entry>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DirectoryFault {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("cannot reach the directory: {0}")]
    Unreachable(String),
    #[error("TLS failed: {0}")]
    Tls(String),
    #[error("the directory did not answer in time")]
    Timeout,
    #[error("the directory refused the operation (result code {code}): {message}")]
    Refused { code: u32, message: String },
}

pub type DirectoryFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, DirectoryFault>> + Send + 'a>>;

pub trait Connector: Send + Sync {
    fn connect<'a>(&'a self, target: &'a Target) -> DirectoryFuture<'a, Box<dyn Session>>;
}

pub trait Session: Send {
    /// Callers never pass an empty password: that is an unauthenticated bind, which many servers
    /// accept.
    fn bind<'a>(&'a mut self, dn: &'a str, password: &'a str) -> DirectoryFuture<'a, ()>;

    fn search<'a>(&'a mut self, request: &'a SearchRequest) -> DirectoryFuture<'a, Found>;

    fn close(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}
