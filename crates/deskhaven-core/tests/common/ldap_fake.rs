//! An in-memory directory behind the `deskhaven_core::ldap::Connector` seam: users with a
//! password and attributes, groups with member DNs, a service account, an outage switch and
//! a log of every operation. Shared with the API crates' tests (`#[path]`), so it only uses
//! deskhaven-core's public API.
//!
//! Searches understand the shapes the service sends: a user search matches users whose
//! `(uid=<escaped value>)` appears in the filter, a group search (base under
//! [`GROUP_BASE`]) matches groups whose escaped member DN appears in it. A bind with an
//! empty password *succeeds* (as an RFC 4513 unauthenticated bind does on many servers),
//! so a test can prove the service never sends one.
// Test helpers outside `#[test]` fns are not covered by clippy.toml allow-*-in-tests.
#![allow(clippy::unwrap_used, clippy::panic, dead_code)]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use deskhaven_core::ldap::{
    Connector, DirectoryFault, DirectoryFuture, Entry, Found, SearchRequest, Session, Target,
    escape_filter_value,
};

pub const BASE: &str = "dc=example,dc=org";
pub const USER_BASE: &str = "ou=people,dc=example,dc=org";
pub const GROUP_BASE: &str = "ou=groups,dc=example,dc=org";
pub const SERVICE_DN: &str = "cn=svc,dc=example,dc=org";
pub const SERVICE_PASSWORD: &str = "svc-Secret-77";
pub const USER_PASSWORD: &str = "dir-Password-31";

pub fn user_dn(uid: &str) -> String {
    format!("uid={uid},{USER_BASE}")
}

pub fn group_dn(cn: &str) -> String {
    format!("cn={cn},{GROUP_BASE}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Connect(Target),
    Bind { dn: String, password_len: usize },
    Search(SearchRequest),
    Close,
}

#[derive(Clone, Debug)]
pub struct FakeUser {
    pub dn: String,
    pub password: String,
    pub attrs: BTreeMap<String, Vec<String>>,
}

#[derive(Default)]
struct State {
    down: bool,
    /// Connects never complete (a directory that accepts TCP and then stalls).
    hang: bool,
    /// User binds fail with this LDAP result code (e.g. 51 busy).
    bind_refusal: Option<u32>,
    users: Vec<FakeUser>,
    /// Group DN -> member DNs.
    groups: BTreeMap<String, Vec<String>>,
    log: Vec<Op>,
}

/// Cheap to clone: clones share the directory.
#[derive(Clone, Default)]
pub struct FakeDirectory {
    state: Arc<Mutex<State>>,
}

impl FakeDirectory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn connector(&self) -> Arc<dyn Connector> {
        Arc::new(self.clone())
    }

    /// `uid` with [`USER_PASSWORD`], `mail` `<uid>@example.org`, `cn` "User <uid>".
    pub fn add_user(&self, uid: &str) -> String {
        let mut attrs = BTreeMap::new();
        attrs.insert("uid".to_owned(), vec![uid.to_owned()]);
        attrs.insert("mail".to_owned(), vec![format!("{uid}@example.org")]);
        attrs.insert("cn".to_owned(), vec![format!("User {uid}")]);
        self.put_user(FakeUser {
            dn: user_dn(uid),
            password: USER_PASSWORD.to_owned(),
            attrs,
        })
    }

    pub fn put_user(&self, user: FakeUser) -> String {
        let dn = user.dn.clone();
        let mut state = self.state.lock().unwrap();
        state.users.retain(|u| u.dn != dn);
        state.users.push(user);
        dn
    }

    pub fn set_attr(&self, dn: &str, attr: &str, values: &[&str]) {
        let mut state = self.state.lock().unwrap();
        let user = state.users.iter_mut().find(|u| u.dn == dn).unwrap();
        let values = values.iter().map(|v| (*v).to_owned()).collect();
        user.attrs.insert(attr.to_owned(), values);
    }

    pub fn set_members(&self, group: &str, members: &[&str]) {
        let members = members.iter().map(|m| (*m).to_owned()).collect();
        self.state
            .lock()
            .unwrap()
            .groups
            .insert(group.to_owned(), members);
    }

    pub fn set_down(&self, down: bool) {
        self.state.lock().unwrap().down = down;
    }

    pub fn set_hang(&self, hang: bool) {
        self.state.lock().unwrap().hang = hang;
    }

    pub fn set_bind_refusal(&self, code: Option<u32>) {
        self.state.lock().unwrap().bind_refusal = code;
    }

    pub fn log(&self) -> Vec<Op> {
        self.state.lock().unwrap().log.clone()
    }

    pub fn clear_log(&self) {
        self.state.lock().unwrap().log.clear();
    }

    pub fn connects(&self) -> usize {
        self.count(|op| matches!(op, Op::Connect(_)))
    }

    /// DNs bound, in order.
    pub fn binds(&self) -> Vec<String> {
        self.log()
            .into_iter()
            .filter_map(|op| match op {
                Op::Bind { dn, .. } => Some(dn),
                _ => None,
            })
            .collect()
    }

    pub fn searches(&self) -> Vec<SearchRequest> {
        self.log()
            .into_iter()
            .filter_map(|op| match op {
                Op::Search(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    fn count(&self, f: impl Fn(&Op) -> bool) -> usize {
        self.state
            .lock()
            .unwrap()
            .log
            .iter()
            .filter(|op| f(op))
            .count()
    }
}

impl Connector for FakeDirectory {
    fn connect<'a>(&'a self, target: &'a Target) -> DirectoryFuture<'a, Box<dyn Session>> {
        Box::pin(async move {
            let hang = {
                let mut state = self.state.lock().unwrap();
                state.log.push(Op::Connect(target.clone()));
                if state.down {
                    return Err(DirectoryFault::Unreachable("connection refused".to_owned()));
                }
                state.hang
            };
            if hang {
                std::future::pending::<()>().await;
            }
            let session: Box<dyn Session> = Box::new(FakeSession {
                state: Arc::clone(&self.state),
            });
            Ok(session)
        })
    }
}

struct FakeSession {
    state: Arc<Mutex<State>>,
}

impl Session for FakeSession {
    fn bind<'a>(&'a mut self, dn: &'a str, password: &'a str) -> DirectoryFuture<'a, ()> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.log.push(Op::Bind {
                dn: dn.to_owned(),
                password_len: password.len(),
            });
            if state.down {
                return Err(DirectoryFault::Unreachable("connection reset".to_owned()));
            }
            let service = dn == SERVICE_DN && password == SERVICE_PASSWORD;
            if let (false, Some(code)) = (service, state.bind_refusal) {
                let message = "refused".to_owned();
                return Err(DirectoryFault::Refused { code, message });
            }
            let user = state
                .users
                .iter()
                .any(|u| u.dn == dn && u.password == password);
            if service || user || password.is_empty() {
                Ok(())
            } else {
                Err(DirectoryFault::InvalidCredentials)
            }
        })
    }

    fn search<'a>(&'a mut self, request: &'a SearchRequest) -> DirectoryFuture<'a, Found> {
        Box::pin(async move {
            let mut state = self.state.lock().unwrap();
            state.log.push(Op::Search(request.clone()));
            if state.down {
                return Err(DirectoryFault::Timeout);
            }
            let mut entries = if request.base.ends_with(GROUP_BASE) {
                group_entries(&state, request)
            } else {
                user_entries(&state, request)
            };
            let truncated = entries.len() > request.size_limit;
            entries.truncate(request.size_limit);
            Ok(Found { entries, truncated })
        })
    }

    fn close(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.state.lock().unwrap().log.push(Op::Close);
        })
    }
}

fn user_entries(state: &State, request: &SearchRequest) -> Vec<Entry> {
    state
        .users
        .iter()
        .filter(|u| u.dn.ends_with(&request.base))
        .filter(|u| {
            u.attrs.get("uid").into_iter().flatten().any(|uid| {
                let wanted = format!("(uid={})", escape_filter_value(uid.trim())).to_lowercase();
                request.filter.to_lowercase().contains(&wanted)
            })
        })
        .map(|u| {
            let mut all = u.attrs.clone();
            let groups = member_of(state, &u.dn);
            if !groups.is_empty() {
                all.insert("memberof".to_owned(), groups);
            }
            Entry {
                dn: u.dn.clone(),
                attrs: selected(all, &request.attrs),
            }
        })
        .collect()
}

/// RFC 4511 §4.5.1.8: no attributes listed means all user attributes, `1.1` means none.
fn selected(
    all: BTreeMap<String, Vec<String>>,
    wanted: &[String],
) -> BTreeMap<String, Vec<String>> {
    if wanted.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|(key, _)| wanted.iter().any(|w| w.eq_ignore_ascii_case(key)))
        .collect()
}

fn member_of(state: &State, dn: &str) -> Vec<String> {
    state
        .groups
        .iter()
        .filter(|(_, members)| members.iter().any(|m| m == dn))
        .map(|(group, _)| group.clone())
        .collect()
}

fn group_entries(state: &State, request: &SearchRequest) -> Vec<Entry> {
    state
        .groups
        .iter()
        .filter(|(group, _)| group.ends_with(&request.base))
        .filter(|(_, members)| {
            members.iter().any(|m| {
                let escaped = escape_filter_value(m);
                request.filter.contains(&escaped)
            })
        })
        .map(|(group, members)| {
            let mut all = BTreeMap::new();
            all.insert("member".to_owned(), members.clone());
            Entry {
                dn: group.clone(),
                attrs: selected(all, &request.attrs),
            }
        })
        .collect()
}
