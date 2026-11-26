use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::strategy;
use deskhaven_db::entity::strategy_assignment::TargetType;
use deskhaven_db::time::TimestampMs;

use super::options::{Options, OptionsPatch};
use super::sealing;
use crate::crypto::SecretBox;

/// Each target has at most one strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    Global,
    Group(i64),
    User(i64),
    DeviceGroup(i64),
    Device(i64),
}

impl Target {
    pub(super) fn kind(self) -> TargetType {
        match self {
            Self::Global => TargetType::Global,
            Self::Group(_) => TargetType::Group,
            Self::User(_) => TargetType::User,
            Self::DeviceGroup(_) => TargetType::DeviceGroup,
            Self::Device(_) => TargetType::Device,
        }
    }

    pub(super) fn id(self) -> Option<i64> {
        match self {
            Self::Global => None,
            Self::Group(id) | Self::User(id) | Self::DeviceGroup(id) | Self::Device(id) => Some(id),
        }
    }

    pub(super) fn from_row(kind: TargetType, id: Option<i64>) -> Option<Self> {
        match (kind, id) {
            (TargetType::Global, None) => Some(Self::Global),
            (TargetType::Group, Some(id)) => Some(Self::Group(id)),
            (TargetType::User, Some(id)) => Some(Self::User(id)),
            (TargetType::DeviceGroup, Some(id)) => Some(Self::DeviceGroup(id)),
            (TargetType::Device, Some(id)) => Some(Self::Device(id)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub target: Target,
    pub strategy_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Strategy {
    pub id: i64,
    pub name: String,
    pub options: Options,
    /// Damaged outside the service; such a strategy is skipped when computing effective strategies until
    /// its options are replaced or it is deleted.
    pub options_corrupt: bool,
    pub note: String,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl Strategy {
    /// Never fails: an unreadable row is shown flagged so the console can still list, repair or delete it.
    pub(super) fn from_row(m: strategy::Model, crypto: &SecretBox) -> Self {
        let decoded = sealing::decode(crypto, m.id, &m.options).map(|o| o.options);
        Self {
            id: m.id,
            name: m.name,
            options_corrupt: decoded.is_none(),
            options: decoded.unwrap_or_default(),
            note: m.note,
            managed_by: m.managed_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        }
    }
}

/// Its options reach every device it applies to in clear, secret ones included (sealed at rest only).
#[derive(Clone, Debug, Default)]
pub struct NewStrategy {
    pub name: String,
    pub options: Options,
    pub note: String,
}

impl NewStrategy {
    pub fn new(name: impl Into<String>, options: Options) -> Self {
        Self {
            name: name.into(),
            options,
            note: String::new(),
        }
    }
}

/// `options_patch` applies on top of `options` in the same transaction, so a patch never overwrites
/// options another writer changed meanwhile.
#[derive(Clone, Debug, Default)]
pub struct StrategyUpdate {
    pub name: Option<String>,
    pub options: Option<Options>,
    pub options_patch: OptionsPatch,
    pub note: Option<String>,
}

impl StrategyUpdate {
    pub(super) fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.options.is_none()
            && self.options_patch.is_empty()
            && self.note.is_none()
    }
}
