//! LDAP service tests: the real service on the test database, talking to the in-memory
//! [`FakeDirectory`], with one directory configured for it.

use std::sync::Arc;

use deskhaven_core::crypto::{MasterKey, SecretBox};
use deskhaven_core::groups::{GroupService, NewGroup};
use deskhaven_core::ldap::{Connector, Directory, LdapService, NewDirectory};
use deskhaven_core::users::{
    AuthError, Authenticated, Credential, HashParams, NewUser, Password, PasswordHasher, User,
    UserService,
};
use deskhaven_db::testing::TestDb;

use super::ldap_fake::{FakeDirectory, GROUP_BASE, SERVICE_DN, SERVICE_PASSWORD, USER_BASE};
use super::{clock, settings};

pub const LOCAL_PASSWORD: &str = "tundra-Lamp-42";
const FAST: HashParams = HashParams {
    memory_kib: 1024,
    iterations: 1,
    parallelism: 1,
};

pub struct Env {
    pub ldap: LdapService,
    pub users: UserService,
    pub groups: GroupService,
    pub fake: FakeDirectory,
    pub directory: Directory,
}

/// A directory for the fake: service bind, `uid` usernames, `mail`, `cn`, groups under
/// [`GROUP_BASE`] from `memberOf`, no admin group.
pub fn new_directory() -> NewDirectory {
    let mut new = NewDirectory::new("corp", "ldaps://ldap.example.org", USER_BASE);
    new.bind_dn = Some(SERVICE_DN.to_owned());
    new.bind_password = Some(SERVICE_PASSWORD.to_owned());
    new.user_filter = "(objectClass=inetOrgPerson)".to_owned();
    new.email_attr = Some("mail".to_owned());
    new.display_name_attr = Some("cn".to_owned());
    new.group_base_dn = Some(GROUP_BASE.to_owned());
    new
}

pub fn service(
    db: &TestDb,
    users: &UserService,
    groups: &GroupService,
    connector: Arc<dyn Connector>,
) -> LdapService {
    let crypto = Arc::new(SecretBox::new(&MasterKey::from_bytes(&[7; 32])));
    LdapService::new(
        db.conn.clone(),
        crypto,
        users.clone(),
        groups.clone(),
        clock(),
        connector,
    )
}

impl Env {
    pub async fn new(db: &TestDb) -> Self {
        Self::with(db, |_| {}).await
    }

    /// The directory adjusted by `adjust` before creation.
    pub async fn with(db: &TestDb, adjust: impl FnOnce(&mut NewDirectory)) -> Self {
        let settings = settings(db).await;
        let hasher = PasswordHasher::new(FAST).unwrap();
        let users = UserService::with_clock(db.conn.clone(), settings, hasher, clock());
        let groups = GroupService::new(db.conn.clone());
        let fake = FakeDirectory::new();
        let ldap = service(db, &users, &groups, fake.connector());
        let mut new = new_directory();
        adjust(&mut new);
        let directory = ldap.create_directory(new).await.unwrap();
        Self {
            ldap,
            users,
            groups,
            fake,
            directory,
        }
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<Authenticated, AuthError> {
        self.ldap
            .authenticate_password(username, &Password::new(password))
            .await
    }

    pub async fn local_user(&self, username: &str) -> User {
        let credential = Credential::Local {
            password: Password::new(LOCAL_PASSWORD),
        };
        self.users
            .create(NewUser::new(username, credential))
            .await
            .unwrap()
    }

    pub async fn group(&self, name: &str) -> i64 {
        self.groups.create(NewGroup::new(name)).await.unwrap().id
    }

    pub async fn group_names(&self, user_id: i64) -> Vec<String> {
        let groups = self.groups.groups_of_user(user_id).await.unwrap();
        groups.into_iter().map(|g| g.name).collect()
    }
}
