//! Directory management (04 §3 `ldap_directories`, 06 §3.5, §5): validation (TLS by
//! default, no link-local or metadata addresses, parseable filters, attribute names), the
//! sealed bind password (re-entered when the connection changes), one enabled directory,
//! config-managed rows. Both backends.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use common::ldap::{Env, new_directory};
use common::ldap_fake::SERVICE_PASSWORD;
use deskhaven_core::ldap::{DirectoryError, DirectoryUpdate, NewDirectory};
use deskhaven_db::entity::ldap_directory;
use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait};

/// A change to a valid directory input.
type Adjust<'a> = &'a dyn Fn(&mut NewDirectory);

async fn stored(db: &TestDb, id: i64) -> ldap_directory::Model {
    ldap_directory::Entity::find_by_id(id)
        .one(&db.conn)
        .await
        .unwrap()
        .unwrap()
}

fn invalid_field(result: Result<impl std::fmt::Debug, DirectoryError>) -> &'static str {
    match result {
        Err(DirectoryError::Invalid { field, .. }) => field,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn the_bind_password_is_sealed_and_never_shown() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;

        let row = stored(db, env.directory.id).await;
        let sealed = row.bind_password_enc.unwrap();
        let settings = env.ldap.stored_settings(env.directory.id).await.unwrap();

        assert!(sealed.starts_with("v1:"), "{}", db.backend);
        assert!(!sealed.contains(SERVICE_PASSWORD));
        assert!(env.directory.has_bind_password);
        let shown = format!("{:?} {:?} {settings:?}", env.directory, new_directory());
        assert!(!shown.contains(SERVICE_PASSWORD), "{shown}");
        // Sealed under its own row: moved to another row, it does not open.
        let mut other = new_directory();
        other.name = "other".to_owned();
        other.enabled = false;
        let other = env.ldap.create_directory(other).await.unwrap();
        let mut moved: ldap_directory::ActiveModel = stored(db, other.id).await.into();
        moved.bind_password_enc = Set(Some(sealed));
        moved.update(&db.conn).await.unwrap();
        assert!(matches!(
            env.ldap.stored_settings(other.id).await,
            Err(DirectoryError::Crypto(_))
        ));
    })
    .await;
}

#[tokio::test]
async fn plain_ldap_needs_start_tls_except_on_loopback() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |d| d.enabled = false).await;
        let with = |name: &str, url: &str, start_tls: bool| {
            let mut new = new_directory();
            new.name = name.to_owned();
            new.url = url.to_owned();
            new.start_tls = start_tls;
            new.enabled = false;
            new
        };

        let plain = env
            .ldap
            .create_directory(with("a", "ldap://ldap.example.org", false))
            .await;
        let start_tls = env
            .ldap
            .create_directory(with("b", "ldap://ldap.example.org:389", true))
            .await;
        let loopback = env
            .ldap
            .create_directory(with("c", "ldap://127.0.0.1:3389", false))
            .await;
        let ldaps_and_start_tls = env
            .ldap
            .create_directory(with("d", "ldaps://ldap.example.org", true))
            .await;

        assert_eq!(invalid_field(plain), "url", "{}", db.backend);
        assert!(start_tls.unwrap().start_tls);
        assert!(loopback.is_ok());
        assert_eq!(invalid_field(ldaps_and_start_tls), "start_tls");
        for bad in [
            "https://ldap.example.org",
            "ldapi:///var/run/slapd.sock",
            "ldaps://user:pw@ldap.example.org",
            "ldaps://ldap.example.org/dc=x?uid",
            "ldaps://",
            // Link-local and cloud metadata addresses (as with OIDC).
            "ldaps://169.254.169.254",
            "ldaps://0xa9fea9fe",
            "ldaps://169.254.43518",
            "ldap://[fe80::1]:389",
            "ldaps://[::ffff:169.254.1.1]",
            "ldaps://[fd00:ec2::254]",
            // ldap3 cannot verify a certificate for a bracketed IPv6 literal.
            "ldaps://[2001:db8::1]:636",
        ] {
            let result = env.ldap.create_directory(with("e", bad, false)).await;
            assert_eq!(invalid_field(result), "url", "{bad}");
        }
    })
    .await;
}

#[tokio::test]
async fn filters_dns_and_attribute_names_are_validated() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::with(db, |d| d.enabled = false).await;
        let try_with = async |adjust: Adjust| {
            let mut new = new_directory();
            new.name = "x".to_owned();
            new.enabled = false;
            adjust(&mut new);
            env.ldap.create_directory(new).await
        };

        let cases: [(Adjust, &str); 9] = [
            (
                &|d| d.user_filter = "objectClass=person".to_owned(),
                "user_filter",
            ),
            (
                &|d| d.user_filter = "(&(objectClass=person)".to_owned(),
                "user_filter",
            ),
            (
                &|d| d.group_filter = Some("(member={user_dn}".to_owned()),
                "group_filter",
            ),
            (&|d| d.username_attr = "uid)(cn".to_owned(), "username_attr"),
            (&|d| d.email_attr = Some("mail*".to_owned()), "email_attr"),
            (&|d| d.user_base_dn = "  ".to_owned(), "user_base_dn"),
            (&|d| d.user_base_dn = "ou=a\nb".to_owned(), "user_base_dn"),
            (&|d| d.bind_password = None, "bind_password"),
            (
                &|d| {
                    d.group_base_dn = None;
                    d.group_filter = Some("(member={user_dn})".to_owned());
                },
                "group_base_dn",
            ),
        ];
        for (adjust, field) in cases {
            assert_eq!(
                invalid_field(try_with(adjust).await),
                field,
                "{}",
                db.backend
            );
        }
        // The server names attributes in results, so an OID would never match.
        let numeric_oid = try_with(&|d| d.username_attr = "0.9.2342.19200300.100.1.1".to_owned());
        assert_eq!(invalid_field(numeric_oid.await), "username_attr");
    })
    .await;
}

#[tokio::test]
async fn only_one_directory_is_enabled_and_names_are_unique() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;

        let second_enabled = env.ldap.create_directory(NewDirectory {
            name: "second".to_owned(),
            ..new_directory()
        });
        let same_name = env.ldap.create_directory(NewDirectory {
            enabled: false,
            ..new_directory()
        });

        assert!(
            matches!(
                second_enabled.await,
                Err(DirectoryError::Conflict { field: "enabled" })
            ),
            "{}",
            db.backend
        );
        assert!(matches!(
            same_name.await,
            Err(DirectoryError::Conflict { field: "name" })
        ));
    })
    .await;
}

#[tokio::test]
async fn updates_change_fields_and_the_password_and_config_rows_are_read_only() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let id = env.directory.id;

        let changed = env
            .ldap
            .update_directory(
                id,
                DirectoryUpdate {
                    bind_dn: Some(None),
                    bind_password: Some(None),
                    admin_group_dn: Some(Some("cn=admins,dc=example,dc=org".to_owned())),
                    ..DirectoryUpdate::default()
                },
            )
            .await
            .unwrap();
        let half = env
            .ldap
            .update_directory(
                id,
                DirectoryUpdate {
                    bind_dn: Some(Some("cn=svc,dc=example,dc=org".to_owned())),
                    ..DirectoryUpdate::default()
                },
            )
            .await;
        let mut managed: ldap_directory::ActiveModel = stored(db, id).await.into();
        managed.managed_by = Set(ManagedBy::Config);
        managed.update(&db.conn).await.unwrap();
        let read_only = env
            .ldap
            .update_directory(id, DirectoryUpdate::default())
            .await;
        let delete = env.ldap.delete_directory(id).await;

        assert!(!changed.has_bind_password, "{}", db.backend);
        assert_eq!(changed.bind_dn, None);
        assert_eq!(stored(db, id).await.bind_password_enc, None);
        assert_eq!(
            changed.admin_group_dn.as_deref(),
            Some("cn=admins,dc=example,dc=org")
        );
        assert_eq!(invalid_field(half), "bind_password");
        assert!(matches!(read_only, Err(DirectoryError::ManagedByConfig)));
        assert!(matches!(delete, Err(DirectoryError::ManagedByConfig)));
    })
    .await;
}

#[tokio::test]
async fn changing_where_the_bind_password_goes_needs_it_entered_again() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let id = env.directory.id;
        // Each of these would send the stored password somewhere else, or unprotected.
        let redirects = [
            DirectoryUpdate {
                url: Some("ldaps://attacker.example.net".to_owned()),
                ..DirectoryUpdate::default()
            },
            DirectoryUpdate {
                tls_verify: Some(false),
                ..DirectoryUpdate::default()
            },
            DirectoryUpdate {
                url: Some("ldap://ldap.example.org".to_owned()),
                start_tls: Some(true),
                ..DirectoryUpdate::default()
            },
            DirectoryUpdate {
                bind_dn: Some(Some("cn=other,dc=example,dc=org".to_owned())),
                ..DirectoryUpdate::default()
            },
        ];
        for update in redirects {
            let refused = env.ldap.update_directory(id, update.clone()).await;
            assert_eq!(
                invalid_field(refused),
                "bind_password",
                "{}: {update:?}",
                db.backend
            );
            let with_password = DirectoryUpdate {
                bind_password: Some(Some("re-entered".to_owned())),
                ..update
            };
            env.ldap.update_directory(id, with_password).await.unwrap();
        }
        // Unchanged connection fields (or other fields) keep the stored password.
        let same_url = DirectoryUpdate {
            url: Some("ldap://ldap.example.org".to_owned()),
            user_filter: Some("(objectClass=person)".to_owned()),
            ..DirectoryUpdate::default()
        };
        env.ldap.update_directory(id, same_url).await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn directories_are_listed_and_deleted() {
    each_migrated_backend(async |db: &TestDb| {
        let env = Env::new(db).await;
        let mut other = new_directory();
        other.name = "another".to_owned();
        other.enabled = false;
        env.ldap.create_directory(other).await.unwrap();

        let names: Vec<String> = env
            .ldap
            .list_directories()
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.name)
            .collect();
        env.ldap.delete_directory(env.directory.id).await.unwrap();

        assert_eq!(names, ["another", "corp"], "{}", db.backend);
        assert!(matches!(
            env.ldap.get_directory(env.directory.id).await,
            Err(DirectoryError::NotFound)
        ));
    })
    .await;
}
