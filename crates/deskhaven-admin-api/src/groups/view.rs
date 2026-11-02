//! Wire types of the group endpoints.

use deskhaven_core::groups::{Group, GroupSummary, ManagedBy};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Who owns a resource: the console, or the configuration file (read-only here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedByView {
    Ui,
    Config,
}

impl From<ManagedBy> for ManagedByView {
    fn from(m: ManagedBy) -> Self {
        match m {
            ManagedBy::Ui => Self::Ui,
            ManagedBy::Config => Self::Config,
        }
    }
}

/// A user group with its number of members. Timestamps are unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct GroupView {
    pub id: i64,
    pub name: String,
    pub note: String,
    pub managed_by: ManagedByView,
    pub member_count: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl GroupView {
    pub(crate) fn new(group: Group, member_count: u64) -> Self {
        Self {
            id: group.id,
            name: group.name,
            note: group.note,
            managed_by: group.managed_by.into(),
            member_count,
            created_at: group.created_at.timestamp_millis(),
            updated_at: group.updated_at.timestamp_millis(),
        }
    }
}

impl From<GroupSummary> for GroupView {
    fn from(s: GroupSummary) -> Self {
        Self::new(s.group, s.member_count)
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewGroupBody {
    /// 1-64 characters, unique.
    #[garde(length(chars, min = 1, max = 64))]
    pub name: String,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub note: String,
}

/// Changes to a group; absent fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupPatch {
    #[garde(length(chars, min = 1, max = 64))]
    pub name: Option<String>,
    #[garde(length(chars, max = 2000))]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AddMemberBody {
    #[garde(range(min = 1))]
    pub user_id: i64,
}
