//! `POST /users/import`: local accounts from CSV, **all or nothing**.
//!
//! One import is one administrative intent: either every row becomes an account (one
//! transaction, one audit row per account) or none does, and the answer lists every
//! problem at once, by line, so the file can be fixed and sent again. A partial import
//! would leave the administrator to work out which rows landed, and re-sending the file
//! would then fail on the rows that did.
//!
//! - Body: `{"csv": "..."}` (the admin API takes JSON only), at most [`MAX_CSV_BYTES`].
//! - Header row required; columns `username` (required), `display_name`, `email`, `role`
//!   (`user` default, `admin`, `owner`), `note`, in any order. Any other column, `password`
//!   included, is refused: passwords never travel in the file.
//! - At most [`MAX_IMPORT_ROWS`] accounts. Each gets a generated temporary password that
//!   must be changed at first login, returned once in the success response.
//! - Row problems: 422 `validation_failed` with `details.rows` ([`ImportRowError`]): field
//!   and a stable code, never the field's value.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use deskhaven_core::state::AppState;
use deskhaven_core::users::{BatchError, Credential, NewUser, Password, Role, RowError, UserError};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::csv::{self, Record};
use super::rules::Acting;
use super::view::{ShownSecret, UserView};
use crate::domain_error::{OWNER_REQUIRED, invalid_field};
use crate::{AdminApiError, AuditCtx, Audited, ConsoleUser, ErrorCode, ValidJson};

/// Largest accepted CSV text (bytes).
pub const MAX_CSV_BYTES: usize = 64 * 1024;
/// Most accounts one import creates. Each needs an Argon2id hash (default parameters:
/// ~19 ms on one core of a 2021 server CPU, so 100 rows ~2 s single-threaded); 100 keeps an
/// import well inside the request deadline on a CPU ten times slower with a single slot.
pub const MAX_IMPORT_ROWS: usize = 100;
const COLUMNS: [&str; 5] = ["username", "display_name", "email", "role", "note"];

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportBody {
    /// The CSV text, header row first.
    #[garde(length(max = MAX_CSV_BYTES))]
    #[schema(max_length = 65536)]
    pub csv: String,
}

/// One created account and its temporary password (shown only here).
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportedUser {
    /// Line of the row in the CSV text.
    pub line: usize,
    pub user: UserView,
    #[schema(value_type = String)]
    pub temporary_password: ShownSecret,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportResult {
    pub created: Vec<ImportedUser>,
}

/// A rejected row: its line, the offending column (if one) and a stable code: `columns`
/// (wrong number of fields), `invalid`, `taken` (by an account or an earlier row),
/// `owner_required`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ImportRowError {
    pub line: usize,
    pub field: Option<String>,
    pub code: String,
}

/// `details` of a 422 import response with row problems.
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportRejection {
    pub rows: Vec<ImportRowError>,
}

/// The error envelope of an import refused for row problems (`code` is
/// `validation_failed`).
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportRejected {
    pub error: ImportRejectedBody,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportRejectedBody {
    pub code: ErrorCode,
    pub message: String,
    pub details: ImportRejection,
}

impl ImportRowError {
    fn new(line: usize, field: Option<&str>, code: &str) -> Self {
        Self {
            line,
            field: field.map(str::to_owned),
            code: code.to_owned(),
        }
    }
}

/// Creates local accounts from CSV, all or nothing (audited `user.import`, one row each).
#[utoipa::path(
    post,
    path = "/users/import",
    tag = "users",
    operation_id = "import_users",
    security(("console_session" = ["admin"])),
    request_body = ImportBody,
    responses(
        (status = 200, description = "Every row created; temporary passwords shown only here", body = ImportResult),
        (status = 422, description = "Nothing created: row problems (`details.rows`), or a malformed CSV (`details.fields.csv`, plain error envelope)", body = ImportRejected),
    )
)]
pub async fn import_users(
    State(state): State<Arc<AppState>>,
    ConsoleUser(principal): ConsoleUser,
    ctx: AuditCtx,
    ValidJson(body): ValidJson<ImportBody>,
) -> Result<Response, AdminApiError> {
    let acting = Acting::new(&principal, ctx);
    let rows = parse_rows(&body.csv)?;
    let (mut errors, candidates) = admin_checks(&acting, &rows);
    let lines: Vec<usize> = candidates.iter().map(|(line, _)| *line).collect();
    let passwords = candidates
        .iter()
        .map(|_| state.users.temporary_password())
        .collect::<Result<Vec<Password>, _>>()?;
    let news = |rows: &[(usize, Row)]| -> Vec<NewUser> {
        let pairs = rows.iter().zip(&passwords);
        pairs
            .map(|((_, row), pw)| row.new_user(pw.clone()))
            .collect()
    };
    let problems = state.users.check_batch(news(&candidates)).await?;
    errors.extend(row_errors(&lines, problems)?);
    if !errors.is_empty() {
        return Ok(rejected(errors));
    }
    let batch = news(&candidates)
        .into_iter()
        .map(|new| {
            let guard = acting.user_guard(&state.console_audit, "user.import", None);
            (new, guard)
        })
        .collect();
    let created = match state.users.create_batch(batch).await {
        Ok(created) => created,
        Err(BatchError::Rows(errs)) => return Ok(rejected(row_errors(&lines, errs)?)),
        Err(BatchError::Failed(err)) => return Err(err.into()),
    };
    let created = created
        .into_iter()
        .zip(lines)
        .zip(&passwords)
        .map(|((user, line), password)| ImportedUser {
            line,
            user: user.into(),
            temporary_password: ShownSecret::new(password),
        })
        .collect();
    Ok((Audited, Json(ImportResult { created })).into_response())
}

/// The fields of one data row, by column.
#[derive(Debug, Clone, Default)]
struct Row {
    username: String,
    display_name: String,
    email: Option<String>,
    role: String,
    note: String,
}

impl Row {
    fn role(&self) -> Option<Role> {
        match self.role.trim().to_ascii_lowercase().as_str() {
            "" | "user" => Some(Role::User),
            "admin" => Some(Role::Admin),
            "owner" => Some(Role::Owner),
            _ => None,
        }
    }

    /// The account to create (role already checked by `admin_checks`).
    fn new_user(&self, password: Password) -> NewUser {
        NewUser {
            display_name: self.display_name.clone(),
            email: self.email.clone(),
            role: self.role().unwrap_or(Role::User),
            must_change_password: true,
            note: self.note.clone(),
            ..NewUser::new(self.username.trim(), Credential::Local { password })
        }
    }
}

/// Header and data rows, or why the text is unusable as a whole.
/// A data row by line: its fields, or why its shape is wrong.
type ParsedRow = (usize, Result<Row, ImportRowError>);

fn parse_rows(text: &str) -> Result<Vec<ParsedRow>, AdminApiError> {
    let records = csv::parse(text).map_err(|err| invalid_field("csv", err))?;
    let (header, data) = records
        .split_first()
        .ok_or_else(|| invalid_field("csv", "no header row"))?;
    let columns = header_columns(header)?;
    if data.is_empty() {
        return Err(invalid_field("csv", "no data rows"));
    }
    if data.len() > MAX_IMPORT_ROWS {
        let reason = format!("at most {MAX_IMPORT_ROWS} rows per import");
        return Err(invalid_field("csv", reason));
    }
    Ok(data.iter().map(|r| (r.line, row(&columns, r))).collect())
}

fn header_columns(header: &Record) -> Result<Vec<&'static str>, AdminApiError> {
    let mut columns: Vec<&'static str> = Vec::with_capacity(header.fields.len());
    for name in &header.fields {
        let name = name.trim().to_ascii_lowercase();
        let known = COLUMNS.iter().find(|c| **c == name).copied();
        let column = known.ok_or_else(|| {
            let reason = format!("unsupported column (allowed: {})", COLUMNS.join(", "));
            invalid_field("csv", reason)
        })?;
        if columns.contains(&column) {
            return Err(invalid_field(
                "csv",
                format!("column {column} appears twice"),
            ));
        }
        columns.push(column);
    }
    if !columns.contains(&"username") {
        return Err(invalid_field("csv", "the username column is required"));
    }
    Ok(columns)
}

fn row(columns: &[&'static str], record: &Record) -> Result<Row, ImportRowError> {
    if record.fields.len() != columns.len() {
        return Err(ImportRowError::new(record.line, None, "columns"));
    }
    let mut row = Row::default();
    for (column, value) in columns.iter().zip(&record.fields) {
        match *column {
            "username" => row.username = value.clone(),
            "display_name" => row.display_name = value.clone(),
            "email" => row.email = Some(value.trim().to_owned()).filter(|e| !e.is_empty()),
            "role" => row.role = value.clone(),
            _ => row.note = value.clone(),
        }
    }
    Ok(row)
}

/// Rows the API refuses before the core sees them (shape, role), and the remaining rows.
fn admin_checks(acting: &Acting, rows: &[ParsedRow]) -> (Vec<ImportRowError>, Vec<(usize, Row)>) {
    let mut errors = Vec::new();
    let mut candidates = Vec::new();
    for (line, parsed) in rows {
        let row = match parsed {
            Ok(row) => row,
            Err(err) => {
                errors.push(err.clone());
                continue;
            }
        };
        match row.role() {
            None => errors.push(ImportRowError::new(*line, Some("role"), "invalid")),
            Some(Role::Owner) if acting.role != Role::Owner => {
                errors.push(ImportRowError::new(*line, Some("role"), OWNER_REQUIRED));
            }
            Some(_) => candidates.push((*line, row.clone())),
        }
    }
    (errors, candidates)
}

/// A core row problem, by CSV line.
/// A row's own problem as a report line; anything else (the actor no longer an admin, a
/// database failure) fails the whole request with its own error, not as an `invalid` row.
fn row_error(lines: &[usize], err: RowError) -> Result<ImportRowError, AdminApiError> {
    let line = lines.get(err.index).copied().unwrap_or(0);
    let (field, code) = match err.error {
        UserError::Invalid { field, .. } => (field, "invalid"),
        UserError::Conflict { field } => (field, "taken"),
        UserError::Refused(r) if r.code == OWNER_REQUIRED => ("role", OWNER_REQUIRED),
        other => return Err(other.into()),
    };
    Ok(ImportRowError::new(line, Some(field), code))
}

fn row_errors(lines: &[usize], errs: Vec<RowError>) -> Result<Vec<ImportRowError>, AdminApiError> {
    errs.into_iter().map(|e| row_error(lines, e)).collect()
}

fn rejected(mut rows: Vec<ImportRowError>) -> Response {
    rows.sort_by_key(|e| e.line);
    tracing::debug!(rows = rows.len(), "account import rejected");
    let body = ImportRejected {
        error: ImportRejectedBody {
            code: ErrorCode::ValidationFailed,
            message: "import rejected; nothing was created".to_owned(),
            details: ImportRejection { rows },
        },
    };
    (StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response()
}
