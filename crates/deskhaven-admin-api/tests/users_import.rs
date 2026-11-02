//! `POST /users/import`: all or nothing, a per-row report of every problem, bounded size
//! and row count, no password column, temporary passwords shown once and never audited.
//! Runs on SQLite and PostgreSQL.
#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use axum::http::Method;
use common::{Console, Harness, harness};
use deskhaven_admin_api::{AdminRoutes, MAX_CSV_BYTES, MAX_IMPORT_ROWS};
use deskhaven_core::pagination::PageRequest;
use deskhaven_core::users::{Password, Role, UserQuery};
use deskhaven_db::testing::{TestDb, each_migrated_backend};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

const IMPORT: &str = "/admin/api/v1/users/import";

async fn import(h: &Harness, console: &Console, csv: &str) -> common::Reply {
    h.write(Method::POST, IMPORT, console, json!({ "csv": csv }))
        .await
}

async fn user_count(h: &Harness) -> u64 {
    let query = UserQuery {
        page: PageRequest::default(),
        ..UserQuery::default()
    };
    h.state.users.list(&query).await.unwrap().total
}

fn rows(reply: &common::Reply) -> Vec<(u64, Option<&str>, &str)> {
    let rows = reply.body["error"]["details"]["rows"].as_array().unwrap();
    rows.iter()
        .map(|r| {
            (
                r["line"].as_u64().unwrap(),
                r["field"].as_str(),
                r["code"].as_str().unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn a_clean_file_creates_every_account_with_a_temporary_password() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;
        let csv = "\u{feff}Username,email,role,display_name,note\r\n\
                   ann,ann@example.com,admin,Ann A,\"first, row\"\r\n\
                   ben,,,,\r\n";
        let reply = import(&h, &admin, csv).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        let created = reply.body["created"].as_array().unwrap();
        assert_eq!(created.len(), 2);
        let ann = &created[0];
        assert_eq!(
            (ann["line"].as_u64(), ann["user"]["role"].as_str()),
            (Some(2), Some("admin"))
        );
        assert_eq!(ann["user"]["note"], "first, row");
        assert_eq!(created[1]["user"]["role"], "user");
        let mut secrets = Vec::new();
        for (row, name) in created.iter().zip(["ann", "ben"]) {
            assert_eq!(row["user"]["must_change_password"], true);
            let temporary = row["temporary_password"].as_str().unwrap();
            let password = Password::new(temporary);
            let auth = h.state.users.authenticate_local(name, &password);
            assert!(auth.await.is_ok(), "{name}");
            secrets.push(temporary.to_owned());
        }

        let audits = h.audit_rows().await;
        let actions: Vec<_> = audits.iter().map(|r| r.action.as_str()).collect();
        assert_eq!(actions, ["user.import", "user.import"]);
        for row in &audits {
            let text = row.after.as_deref().unwrap();
            assert!(secrets.iter().all(|s| !text.contains(s.as_str())));
        }
    })
    .await;
}

#[tokio::test]
async fn any_bad_row_rejects_the_whole_file_with_every_problem() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;
        h.create_user("taken", Role::User).await;
        let before = user_count(&h).await;
        let csv = "username,email,role\n\
                   fine,fine@example.com,user\n\
                   bad name,,\n\
                   TAKEN,,\n\
                   dup,d1@example.com,\n\
                   Dup,d2@example.com,\n\
                   boss,,owner\n\
                   who,,root\n\
                   short,row\n\
                   mail,not-an-email,\n";
        let reply = import(&h, &admin, csv).await;
        assert_eq!(
            (reply.status.as_u16(), reply.code()),
            (422, "validation_failed")
        );
        assert_eq!(
            rows(&reply),
            [
                (3, Some("username"), "invalid"),
                (4, Some("username"), "taken"),
                (6, Some("username"), "taken"),
                (7, Some("role"), "owner_required"),
                (8, Some("role"), "invalid"),
                (9, None, "columns"),
                (10, Some("email"), "invalid"),
            ]
        );
        assert_eq!(user_count(&h).await, before, "nothing was created");
        assert!(h.audit_rows().await.is_empty());
        assert!(
            !reply.body.to_string().contains("not-an-email"),
            "values are not echoed"
        );

        // An owner may import owners.
        let owner = h.console("own", Role::Owner).await;
        let reply = import(&h, &owner, "username,role\nboss,owner\n").await;
        assert_eq!(reply.status, 200);
    })
    .await;
}

#[tokio::test]
async fn malformed_files_and_limits_are_refused_as_a_whole() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;
        let field_error = |reply: &common::Reply| -> String {
            assert_eq!(
                (reply.status.as_u16(), reply.code()),
                (422, "validation_failed")
            );
            let reasons = &reply.body["error"]["details"]["fields"]["csv"];
            reasons[0].as_str().unwrap_or_default().to_owned()
        };
        let secret = "Hunter2-very-secret";
        let with_password = format!("username,password\nann,{secret}\n");
        let reply = import(&h, &admin, &with_password).await;
        assert!(field_error(&reply).contains("unsupported column"));
        assert!(!reply.body.to_string().contains(secret));

        for (csv, reason) in [
            ("", "no header row"),
            ("username\n", "no data rows"),
            ("email\nx@example.com\n", "username column is required"),
            ("username,username\na,b\n", "appears twice"),
            ("username\n\"open", "not closed"),
        ] {
            let got = field_error(&import(&h, &admin, csv).await);
            assert!(got.contains(reason), "{csv:?}: {got}");
        }

        let mut too_many = String::from("username\n");
        for i in 0..=MAX_IMPORT_ROWS {
            too_many.push_str(&format!("u{i}\n"));
        }
        assert!(field_error(&import(&h, &admin, &too_many).await).contains("at most"));

        let huge = format!("username,note\nann,{}\n", "x".repeat(MAX_CSV_BYTES));
        let reply = import(&h, &admin, &huge).await;
        assert_eq!(
            (reply.status.as_u16(), reply.code()),
            (422, "validation_failed")
        );
        let fields: &Value = &reply.body["error"]["details"]["fields"];
        assert!(fields.get("csv").is_some());
        assert_eq!(user_count(&h).await, 1);
    })
    .await;
}

#[tokio::test]
async fn a_file_of_the_maximum_row_count_imports() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;
        let mut csv = String::from("username\n");
        for i in 0..MAX_IMPORT_ROWS {
            csv.push_str(&format!("bulk{i}\n"));
        }
        let reply = import(&h, &admin, &csv).await;
        assert_eq!(reply.status, 200, "{}", reply.body);
        let created = reply.body["created"].as_array().unwrap();
        assert_eq!(created.len(), MAX_IMPORT_ROWS);
        assert_eq!(created[7]["user"]["username"], "bulk7");
        let password = Password::new(created[7]["temporary_password"].as_str().unwrap());
        let login = h.state.users.authenticate_local("bulk7", &password).await;
        assert!(login.is_ok());
    })
    .await;
}

#[tokio::test]
async fn a_database_failure_during_import_is_a_server_error_not_a_bad_row() {
    each_migrated_backend(async |db: &TestDb| {
        let h = harness(db, AdminRoutes::new()).await;
        let admin = h.console("adm", Role::Admin).await;
        // The accounts insert fine; their audit rows cannot be written.
        h.state
            .db
            .execute_unprepared("ALTER TABLE console_audit RENAME TO console_audit_gone")
            .await
            .unwrap();

        let reply = import(&h, &admin, "username\nann\nbob\n").await;
        assert_eq!((reply.status.as_u16(), reply.code()), (500, "internal"));
        assert!(reply.body["error"]["details"]["rows"].is_null());
        assert_eq!(user_count(&h).await, 1);
    })
    .await;
}
