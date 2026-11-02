use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::strategy;
use deskhaven_db::entity::strategy_assignment::TargetType;
use deskhaven_db::time::TimestampMs;

use super::StrategyError;
use super::options::{self, Options};

/// What a strategy is assigned to. Each target has at most one strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// Every device; the lowest priority.
    Global,
    /// Members of a user group (`groups.id`).
    Group(i64),
    /// Devices owned by a user (`users.id`).
    User(i64),
    /// Devices in a device group (`device_groups.id`).
    DeviceGroup(i64),
    /// One device (`devices.id`); the highest priority.
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

    /// `None` exactly for [`Target::Global`] (the schema stores NULL there).
    pub(super) fn id(self) -> Option<i64> {
        match self {
            Self::Global => None,
            Self::Group(id) | Self::User(id) | Self::DeviceGroup(id) | Self::Device(id) => Some(id),
        }
    }

    /// `None` for a row violating the schema's `target_id` check.
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

/// A strategy assigned to a target.
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
    pub note: String,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl TryFrom<strategy::Model> for Strategy {
    type Error = StrategyError;

    fn try_from(m: strategy::Model) -> Result<Self, StrategyError> {
        let options =
            options::decode(&m.options).ok_or(StrategyError::Corrupt { strategy_id: m.id })?;
        Ok(Self {
            id: m.id,
            name: m.name,
            options,
            note: m.note,
            managed_by: m.managed_by,
            created_at: m.created_at,
            updated_at: m.updated_at,
        })
    }
}

/// Input of [`StrategyService::create`](super::StrategyService::create); the strategy is
/// managed in the console (`ui`).
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

/// Fields to change; `None` leaves a field as it is. `options` replaces the whole map.
#[derive(Clone, Debug, Default)]
pub struct StrategyUpdate {
    pub name: Option<String>,
    pub options: Option<Options>,
    pub note: Option<String>,
}
