//! `config` resources come from the bootstrap config file and are read-only in the UI.

use sea_orm::entity::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Text")]
pub enum ManagedBy {
    #[sea_orm(string_value = "ui")]
    Ui,
    #[sea_orm(string_value = "config")]
    Config,
}
