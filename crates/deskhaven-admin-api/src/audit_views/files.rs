//! `GET /audit/files`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::audit_query::{
    AuditQuery, FileQuery, TransferredFile, parse_stored, stored_name, transferred_files,
};
use deskhaven_core::state::AppState;
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::filter::{MAX_LABEL_CHARS, MAX_Q_CHARS, WindowQuery, known, trimmed};
use crate::{AdminApiError, PageQuery, Paginated, ValidQuery};

/// Filters of the file transfer list.
#[derive(Debug, Clone, Default, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FileFilter {
    /// The reporting (controlled) device's RustDesk id.
    #[garde(length(chars, max = MAX_LABEL_CHARS))]
    #[param(max_length = 256)]
    pub device: Option<String>,
    /// As seen from the reporting device: `send` or `receive`.
    #[garde(length(max = 16))]
    #[param(pattern = "^(send|receive)$")]
    pub direction: Option<String>,
    /// Substring of the path, peer id or address (case-insensitive, literal).
    #[garde(length(chars, max = MAX_Q_CHARS))]
    #[param(max_length = 200)]
    pub q: Option<String>,
}

/// One file of a transfer, as the device named it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TransferredFileView {
    pub name: String,
    /// Bytes.
    pub size: i64,
}

impl From<TransferredFile> for TransferredFileView {
    fn from(file: TransferredFile) -> Self {
        Self {
            name: file.name,
            size: file.size,
        }
    }
}

/// A file transfer; all of it is device self-reported except `id` and `created_at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct FileAuditView {
    pub id: i64,
    /// The reporting (controlled) device's RustDesk id.
    pub device_id: String,
    pub peer_id: String,
    pub conn_id: i64,
    /// `send` or `receive`, as seen from the reporting device.
    pub direction: String,
    pub path: String,
    pub is_file: bool,
    pub file_count: i32,
    /// At most the first ten files.
    pub files: Vec<TransferredFileView>,
    pub ip: Option<String>,
    pub created_at: i64,
}

impl From<deskhaven_core::audit_query::FileRow> for FileAuditView {
    fn from(row: deskhaven_core::audit_query::FileRow) -> Self {
        Self {
            id: row.id,
            device_id: row.device_rustdesk_id,
            peer_id: row.peer_rustdesk_id,
            conn_id: row.conn_id,
            direction: stored_name(&row.direction),
            files: transferred_files(&row.files)
                .into_iter()
                .map(Into::into)
                .collect(),
            path: row.path,
            is_file: row.is_file,
            file_count: row.file_count,
            ip: row.ip,
            created_at: row.created_at.timestamp_millis(),
        }
    }
}

/// File transfers, newest first by default.
#[utoipa::path(
    get,
    path = "/audit/files",
    tag = "audit",
    operation_id = "list_file_audit",
    security(("console_session" = ["admin"])),
    params(PageQuery, WindowQuery, FileFilter),
    responses((status = 200, description = "One page of file transfers", body = Paginated<FileAuditView>))
)]
pub async fn list_file_audit(
    State(state): State<Arc<AppState>>,
    ValidQuery(page): ValidQuery<PageQuery>,
    ValidQuery(window): ValidQuery<WindowQuery>,
    ValidQuery(filter): ValidQuery<FileFilter>,
) -> Result<Json<Paginated<FileAuditView>>, AdminApiError> {
    let query = FileQuery {
        listing: window.listing(page)?,
        device: trimmed(filter.device),
        direction: known("direction", filter.direction.as_deref(), parse_stored)?,
        q: filter.q,
    };
    let found = AuditQuery::new(state.db.clone())
        .file_transfers(&query)
        .await?;
    Ok(Json(Paginated::from_page(found, Into::into)))
}
