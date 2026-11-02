//! Schema migrations (forward-only).
//!
//! Naming: `mYYYYMMDD_NNNNNN_<name>` (timestamp, not a sequence number), registered in order
//! in [`Migrator::migrations`]. One migration body serves both backends; dialect differences
//! branch on `manager.get_database_backend()`.

use sea_orm_migration::{MigrationTrait, MigratorTrait};

mod m20261102_000001_settings;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(m20261102_000001_settings::Migration)]
    }
}
