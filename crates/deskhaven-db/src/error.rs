use sea_orm::DbErr;

/// Errors of the database layer.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// The URL scheme is not one of the supported backends. Only the scheme is kept so
    /// credentials embedded in the URL never reach logs.
    #[error("unsupported database URL scheme `{scheme}` (expected `sqlite:` or `postgres:`)")]
    UnsupportedUrl { scheme: String },

    #[error("failed to connect to the database")]
    Connect(#[source] DbErr),

    #[error("database migration failed")]
    Migration(#[source] DbErr),
}
