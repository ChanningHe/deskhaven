//! The effective strategy of a device with the layer each option comes from (05 §4.3).
//! Values of secret catalog options are withheld (`redacted`).

use deskhaven_core::devices::DeviceRecord;
use deskhaven_core::state::AppState;
use deskhaven_core::strategies::catalog;
use deskhaven_core::strategies::{DeviceInputs, Options, StrategyError, Target};
use serde::Serialize;
use utoipa::ToSchema;

use crate::AdminApiError;

/// What a strategy is assigned to; later layers win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StrategyScopeView {
    /// Every device.
    Global,
    /// A user group the device's owner belongs to.
    Group,
    /// The device's owner.
    User,
    DeviceGroup,
    /// This device only.
    Device,
}

/// One strategy applying to the device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct StrategyLayerView {
    pub scope: StrategyScopeView,
    /// The group, user or device group id; absent for `global`.
    pub target_id: Option<i64>,
    pub strategy_id: i64,
    pub strategy_name: String,
}

/// One option of the merged result and the layer it comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct StrategyOptionView {
    pub key: String,
    /// Empty: the client deletes the option (unless `redacted`).
    pub value: String,
    /// A secret option's value, withheld here (`value` is empty).
    pub redacted: bool,
    pub source: StrategyLayerView,
}

/// The strategy a device ends up with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct EffectiveStrategyView {
    /// Version the clients compare (unix milliseconds).
    pub modified_at: i64,
    /// Strategies that apply, lowest priority first.
    pub layers: Vec<StrategyLayerView>,
    /// The merged options, by name.
    pub options: Vec<StrategyOptionView>,
}

/// A layer with the options of its strategy, for resolving sources.
struct Layer {
    view: StrategyLayerView,
    options: Options,
}

fn scope_of(target: Target) -> (StrategyScopeView, Option<i64>) {
    match target {
        Target::Global => (StrategyScopeView::Global, None),
        Target::Group(id) => (StrategyScopeView::Group, Some(id)),
        Target::User(id) => (StrategyScopeView::User, Some(id)),
        Target::DeviceGroup(id) => (StrategyScopeView::DeviceGroup, Some(id)),
        Target::Device(id) => (StrategyScopeView::Device, Some(id)),
    }
}

/// The effective strategy of `device`. A strategy deleted since the merge is left out of
/// the layers (its options then have no source: they are dropped too).
pub(crate) async fn effective_strategy(
    state: &AppState,
    device: &DeviceRecord,
) -> Result<EffectiveStrategyView, AdminApiError> {
    let inputs = DeviceInputs {
        device_id: device.id,
        user_id: device.owner_user_id,
        device_group_id: device.device_group_id,
        changed_at: None,
    };
    let effective = state
        .strategies
        .effective(&inputs)
        .await
        .map_err(AdminApiError::internal)?;
    let mut layers = Vec::with_capacity(effective.applied.len());
    for assignment in &effective.applied {
        match state.strategies.get(assignment.strategy_id).await {
            Ok(strategy) => {
                let (scope, target_id) = scope_of(assignment.target);
                layers.push(Layer {
                    view: StrategyLayerView {
                        scope,
                        target_id,
                        strategy_id: strategy.id,
                        strategy_name: strategy.name,
                    },
                    options: strategy.options,
                });
            }
            Err(StrategyError::NotFound) => {}
            Err(other) => return Err(AdminApiError::internal(other)),
        }
    }
    let options = effective
        .options
        .into_iter()
        .filter_map(|(key, value)| {
            let source = layers.iter().rev().find(|l| l.options.contains_key(&key))?;
            let redacted = !value.is_empty() && catalog::is_secret(&key);
            Some(StrategyOptionView {
                value: if redacted { String::new() } else { value },
                redacted,
                key,
                source: source.view.clone(),
            })
        })
        .collect();
    Ok(EffectiveStrategyView {
        modified_at: effective.modified_at,
        layers: layers.into_iter().map(|l| l.view).collect(),
        options,
    })
}
