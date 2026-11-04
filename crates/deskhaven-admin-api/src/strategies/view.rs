//! Wire types of the strategy endpoints and the option checks of the console: option names
//! are `[A-Za-z0-9._-]`, and values of catalog options must fit their kind (bool, choice,
//! number). Sizes and control characters are the core service's checks.

use std::collections::BTreeMap;

use deskhaven_core::strategies::catalog;
use deskhaven_core::strategies::{AssignedTarget, KEY_MAX_LEN, StrategySummary, Target};
use garde::Validate;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::AdminApiError;
use crate::domain_error::invalid_field;
use crate::groups::ManagedByView;

/// A strategy. Values of secret options (see the catalog) are never returned: such an
/// option holding a value is listed in `secret_options` instead of `options`. Timestamps
/// are unix milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct StrategyView {
    pub id: i64,
    pub name: String,
    pub note: String,
    pub managed_by: ManagedByView,
    /// Option name to value; an empty value makes the client delete the option.
    pub options: BTreeMap<String, String>,
    /// Secret options that hold a (withheld) value.
    pub secret_options: Vec<String>,
    /// The stored options are damaged; replace them to repair the strategy.
    pub options_corrupt: bool,
    /// Number of targets the strategy is assigned to.
    pub assignment_count: u64,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<StrategySummary> for StrategyView {
    fn from(s: StrategySummary) -> Self {
        let strategy = s.strategy;
        let (secret, options): (BTreeMap<_, _>, BTreeMap<_, _>) = strategy
            .options
            .into_iter()
            .partition(|(key, value)| !value.is_empty() && catalog::is_secret(key));
        Self {
            id: strategy.id,
            name: strategy.name,
            note: strategy.note,
            managed_by: strategy.managed_by.into(),
            options,
            secret_options: secret.into_keys().collect(),
            options_corrupt: strategy.options_corrupt,
            assignment_count: s.assignment_count,
            created_at: strategy.created_at.timestamp_millis(),
            updated_at: strategy.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewStrategyBody {
    /// 1-64 characters, unique.
    #[garde(length(chars, min = 1, max = 64))]
    pub name: String,
    #[serde(default)]
    #[garde(length(chars, max = 2000))]
    pub note: String,
    /// Option name to value (an empty value makes the client delete the option).
    #[serde(default)]
    #[garde(skip)]
    pub options: BTreeMap<String, String>,
}

/// Changes to a strategy; absent fields stay as they are. `options` is a merge patch: a
/// string sets the option, `null` removes it from the strategy, options not named stay.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StrategyPatch {
    #[garde(length(chars, min = 1, max = 64))]
    pub name: Option<String>,
    #[garde(length(chars, max = 2000))]
    pub note: Option<String>,
    #[garde(skip)]
    #[schema(value_type = Option<BTreeMap<String, Option<String>>>)]
    pub options: Option<BTreeMap<String, Option<String>>>,
}

/// What a strategy is assigned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// Every device; no `target_id`.
    Global,
    /// A user group (`groups.id`).
    Group,
    /// The devices a user owns (`users.id`).
    User,
    /// A device group.
    DeviceGroup,
    /// One device (`devices.id`).
    Device,
}

/// An assignment target: a kind and, except for `global`, the id of the user, group,
/// device group or device.
#[derive(Debug, Deserialize, Validate, ToSchema, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct TargetBody {
    #[garde(skip)]
    pub target_type: TargetKind,
    #[garde(inner(range(min = 1)))]
    pub target_id: Option<i64>,
}

impl TargetBody {
    pub(super) fn target(&self) -> Result<Target, AdminApiError> {
        let missing = || invalid_field("target_id", "is required for this target type");
        match (self.target_type, self.target_id) {
            (TargetKind::Global, None) => Ok(Target::Global),
            (TargetKind::Global, Some(_)) => Err(invalid_field(
                "target_id",
                "must be absent for the global target",
            )),
            (TargetKind::Group, id) => id.map(Target::Group).ok_or_else(missing),
            (TargetKind::User, id) => id.map(Target::User).ok_or_else(missing),
            (TargetKind::DeviceGroup, id) => id.map(Target::DeviceGroup).ok_or_else(missing),
            (TargetKind::Device, id) => id.map(Target::Device).ok_or_else(missing),
        }
    }
}

pub(super) fn kind_of(target: Target) -> (TargetKind, Option<i64>) {
    match target {
        Target::Global => (TargetKind::Global, None),
        Target::Group(id) => (TargetKind::Group, Some(id)),
        Target::User(id) => (TargetKind::User, Some(id)),
        Target::DeviceGroup(id) => (TargetKind::DeviceGroup, Some(id)),
        Target::Device(id) => (TargetKind::Device, Some(id)),
    }
}

/// A target the strategy is assigned to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AssignmentView {
    pub target_type: TargetKind,
    pub target_id: Option<i64>,
    /// User name, group or device group name, or the device's RustDesk id; absent for
    /// `global`.
    pub target_name: Option<String>,
}

impl From<AssignedTarget> for AssignmentView {
    fn from(a: AssignedTarget) -> Self {
        let (target_type, target_id) = kind_of(a.target);
        Self {
            target_type,
            target_id,
            target_name: a.name,
        }
    }
}

/// The targets of a strategy, global first, devices last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct AssignmentList {
    pub items: Vec<AssignmentView>,
}

/// Device whose effective strategy to preview.
#[derive(Debug, Deserialize, Validate, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PreviewQuery {
    /// `devices.id`.
    #[garde(range(min = 1))]
    pub device_id: i64,
}

/// Option names are 1-128 of `A-Z a-z 0-9 . _ -`.
fn key_problem(key: &str) -> Option<&'static str> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    match key.chars().count() {
        0 => Some("option names must not be empty"),
        n if n > KEY_MAX_LEN => Some("option names are at most 128 characters"),
        _ if !key.chars().all(allowed) => {
            Some("option names may only contain letters, digits, '.', '_' and '-'")
        }
        _ => None,
    }
}

/// Checks names and, for catalog options, values; `None` values (removals) only by name.
/// The error names the option, never its value.
pub(super) fn check_options<'a>(
    entries: impl IntoIterator<Item = (&'a String, Option<&'a String>)>,
) -> Result<(), AdminApiError> {
    for (key, value) in entries {
        if let Some(reason) = key_problem(key) {
            return Err(invalid_field("options", reason));
        }
        if let Some(reason) = value.and_then(|v| catalog::value_problem(key, v)) {
            return Err(invalid_field(&format!("options.{key}"), reason));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_names_are_restricted_to_a_safe_charset() {
        for good in [
            "enable-keyboard",
            "collapse_toolbar",
            "hideAbTagsPanel",
            "2fa",
            "a.b",
        ] {
            assert_eq!(key_problem(good), None, "{good}");
        }
        for bad in ["", "has space", "a/b", "ü", "a=b", &"k".repeat(129)] {
            assert!(key_problem(bad).is_some(), "{bad}");
        }
    }

    #[test]
    fn catalog_values_are_typed_and_removals_are_not_checked() {
        let (key, yes, maybe) = (
            "enable-keyboard".to_owned(),
            "Y".to_owned(),
            "maybe".to_owned(),
        );
        assert!(check_options([(&key, Some(&yes))]).is_ok());
        assert!(check_options([(&key, None)]).is_ok());
        assert!(check_options([(&key, Some(&maybe))]).is_err());
    }
}
