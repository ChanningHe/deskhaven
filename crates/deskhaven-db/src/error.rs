use sea_orm::DbErr;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// Only the scheme is kept so credentials embedded in the URL never reach logs.
    #[error("unsupported database URL scheme `{scheme}` (expected `sqlite:` or `postgres:`)")]
    UnsupportedUrl { scheme: String },

    #[error("failed to connect to the database")]
    Connect(#[source] DbErr),

    #[error("database migration failed")]
    Migration(#[source] DbErr),

    #[error("failed to read the migration status")]
    MigrationStatus(#[source] DbErr),
}
