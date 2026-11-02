//! SeaORM entities. Conventions (04-data-model §1): time columns are
//! [`TimestampMs`](crate::time::TimestampMs), JSON is stored as `TEXT` (serde in the app layer).

pub mod setting;
pub mod setting_history;
