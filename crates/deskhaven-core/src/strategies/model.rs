use deskhaven_db::entity::managed_by::ManagedBy;
use deskhaven_db::entity::strategy;
use deskhaven_db::entity::strategy_assignment::TargetType;
use deskhaven_db::time::TimestampMs;

use super::options::{Options, OptionsPatch};
use super::sealing;
use crate::crypto::SecretBox;

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
    /// Empty when [`options_corrupt`](Self::options_corrupt).
    pub options: Options,
    /// The stored options are not a `{string: string}` JSON object (damaged outside the
    /// service). Such a strategy is skipped when computing effective strategies (logged at
    /// error level when the cache loads) until its options are replaced or it is deleted.
    pub options_corrupt: bool,
    pub note: String,
    pub managed_by: ManagedBy,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
}

impl Strategy {
    /// The row with its secret options opened. Never fails: a row whose options cannot be
    /// read is shown flagged ([`Strategy::options_corrupt`]) so the console can still list,
    /// repair or delete it.
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

/// Input of [`StrategyService::create`](super::StrategyService::create); the strategy is
/// managed in the console (`ui`). Its options reach every device it applies to in clear,
/// secret ones included (sealed at rest only; see the module documentation).
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

/// Fields to change; `None` leaves a field as it is. `options` replaces the whole map, then
/// `options_patch` changes single options (applied on top, in the same transaction, so a
/// patch never overwrites options another writer changed meanwhile).
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
