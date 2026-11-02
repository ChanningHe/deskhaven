//! Effective strategy of a device (05 §4.3): the assigned strategies that apply to it,
//! overlaid from lowest to highest priority.
//!
//! Layers, lowest first:
//!
//! 1. `global`
//! 2. `group`: one layer per user group the device's owner belongs to that has a strategy,
//!    in ascending group id (a tie between groups: the group created last wins)
//! 3. `user`: the device's owner
//! 4. `device_group`
//! 5. `device`
//!
//! Merging works per option name: a higher layer's value replaces a lower layer's, and an
//! **empty value is a value like any other**. It overrides what lower layers set and stays
//! in the result, because the client deletes an option when it receives it empty (03 §2.1);
//! dropping it here would leave the client with whatever it applied before.

use std::collections::{HashMap, HashSet};

use deskhaven_db::entity::device;
use deskhaven_db::time::TimestampMs;

use super::model::{Assignment, Target};
use super::options::Options;

/// What decides which strategies apply to a device. Built by the caller from the device
/// registry (`From<&device::Model>`), so the strategy service never reads `devices`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeviceInputs {
    /// `devices.id`.
    pub device_id: i64,
    /// `devices.owner_user_id`.
    pub user_id: Option<i64>,
    /// `devices.device_group_id`.
    pub device_group_id: Option<i64>,
    /// When the owner or the device group of the device last changed by an explicit
    /// assignment (deploy / assign), folded into [`EffectiveStrategy::modified_at`] so a
    /// client that moved to another group refetches. `None` = unknown or never.
    pub changed_at: Option<TimestampMs>,
}

impl From<&device::Model> for DeviceInputs {
    /// `changed_at` stays `None`: `devices.updated_at` also moves on every re-registration,
    /// which would make clients refetch for nothing.
    fn from(device: &device::Model) -> Self {
        Self {
            device_id: device.id,
            user_id: device.owner_user_id,
            device_group_id: device.device_group_id,
            changed_at: None,
        }
    }
}

/// The result of merging the strategies that apply to one device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveStrategy {
    /// `config_options` to send. Entries with an empty value are kept: they tell the client
    /// to delete the option.
    pub options: Options,
    /// Version of this result in unix milliseconds. The server value only grows (see
    /// [`StrategyCache`](super::StrategyCache)); send `options` to a client whose stored
    /// value is lower, nothing to one whose value is equal or higher.
    pub modified_at: i64,
    /// The strategies merged, lowest priority first.
    pub applied: Vec<Assignment>,
}

impl EffectiveStrategy {
    /// The options the device ends up with: [`options`](Self::options) without the empty
    /// (delete) entries.
    pub fn live_options(&self) -> impl Iterator<Item = (&str, &str)> {
        let live = self.options.iter().filter(|(_, value)| !value.is_empty());
        live.map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

/// A user group with a strategy and its members.
#[derive(Debug)]
struct GroupLayer {
    group_id: i64,
    strategy_id: i64,
    members: HashSet<i64>,
}

/// All assignments and the options they point at, as of one load. Immutable; replaced as a
/// whole when anything changes.
#[derive(Debug, Default)]
pub(super) struct Snapshot {
    /// Cache version at load time (see [`StrategyCache`](super::StrategyCache)).
    modified_at: i64,
    global: Option<i64>,
    devices: HashMap<i64, i64>,
    device_groups: HashMap<i64, i64>,
    users: HashMap<i64, i64>,
    /// Ascending group id.
    groups: Vec<GroupLayer>,
    /// Strategy id to options, for every assigned strategy.
    options: HashMap<i64, Options>,
}

impl Snapshot {
    /// `members` maps a user group id to its member user ids and need only cover groups that
    /// have an assignment. Assignments whose strategy is missing from `options` are ignored.
    pub(super) fn build(
        modified_at: i64,
        assignments: &[Assignment],
        options: HashMap<i64, Options>,
        mut members: HashMap<i64, HashSet<i64>>,
    ) -> Self {
        let mut snap = Self {
            modified_at,
            options,
            ..Self::default()
        };
        for a in assignments {
            let id = a.strategy_id;
            match a.target {
                Target::Global => snap.global = Some(id),
                Target::Device(target) => drop(snap.devices.insert(target, id)),
                Target::DeviceGroup(target) => drop(snap.device_groups.insert(target, id)),
                Target::User(target) => drop(snap.users.insert(target, id)),
                Target::Group(group_id) => snap.groups.push(GroupLayer {
                    group_id,
                    strategy_id: id,
                    members: members.remove(&group_id).unwrap_or_default(),
                }),
            }
        }
        snap.groups.sort_by_key(|g| g.group_id);
        snap
    }

    /// The assignments that apply to `inputs`, lowest priority first.
    fn layers(&self, inputs: &DeviceInputs) -> Vec<Assignment> {
        let mut layers = Vec::new();
        let mut push = |target: Target, strategy_id: Option<&i64>| {
            let known = strategy_id.filter(|id| self.options.contains_key(id));
            layers.extend(known.map(|&strategy_id| Assignment {
                target,
                strategy_id,
            }));
        };
        push(Target::Global, self.global.as_ref());
        if let Some(user) = inputs.user_id {
            for g in self.groups.iter().filter(|g| g.members.contains(&user)) {
                push(Target::Group(g.group_id), Some(&g.strategy_id));
            }
            push(Target::User(user), self.users.get(&user));
        }
        if let Some(group) = inputs.device_group_id {
            push(Target::DeviceGroup(group), self.device_groups.get(&group));
        }
        push(
            Target::Device(inputs.device_id),
            self.devices.get(&inputs.device_id),
        );
        layers
    }

    pub(super) fn effective(&self, inputs: &DeviceInputs) -> EffectiveStrategy {
        let applied = self.layers(inputs);
        let mut options = Options::new();
        for layer in &applied {
            if let Some(layer_options) = self.options.get(&layer.strategy_id) {
                options.extend(layer_options.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
        }
        EffectiveStrategy {
            options,
            modified_at: stamp(self.modified_at, inputs),
            applied,
        }
    }
}

/// `modified_at` of a device: the cache version, or the device's own `changed_at` if later.
pub(super) fn stamp(version: i64, inputs: &DeviceInputs) -> i64 {
    let changed = inputs.changed_at.map(|t| t.timestamp_millis());
    version.max(changed.unwrap_or(i64::MIN))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GLOBAL: i64 = 1;
    const GROUP_A: i64 = 2;
    const GROUP_B: i64 = 3;
    const USER: i64 = 4;
    const DEVICE_GROUP: i64 = 5;
    const DEVICE: i64 = 6;
    const MODIFIED_AT: i64 = 1_000;

    fn opts(pairs: &[(&str, &str)]) -> Options {
        let own = |(k, v): &(&str, &str)| ((*k).to_owned(), (*v).to_owned());
        pairs.iter().map(own).collect()
    }

    fn at(target: Target, strategy_id: i64) -> Assignment {
        Assignment {
            target,
            strategy_id,
        }
    }

    /// Entity ids are arbitrary here: user 10, device 20, groups 7 and 8, device group 30.
    fn inputs() -> DeviceInputs {
        DeviceInputs {
            device_id: 20,
            user_id: Some(10),
            device_group_id: Some(30),
            changed_at: None,
        }
    }

    fn snapshot(assignments: &[Assignment], strategies: &[(i64, Options)]) -> Snapshot {
        let members = HashMap::from([(7, HashSet::from([10])), (8, HashSet::from([10, 11]))]);
        let options = strategies.iter().cloned().collect();
        Snapshot::build(MODIFIED_AT, assignments, options, members)
    }

    #[test]
    fn higher_priority_overrides_lower_per_option() {
        let snap = snapshot(
            &[
                at(Target::Global, GLOBAL),
                at(Target::Group(7), GROUP_A),
                at(Target::User(10), USER),
                at(Target::DeviceGroup(30), DEVICE_GROUP),
                at(Target::Device(20), DEVICE),
            ],
            &[
                (
                    GLOBAL,
                    opts(&[("a", "global"), ("b", "global"), ("c", "global")]),
                ),
                (
                    GROUP_A,
                    opts(&[("b", "group"), ("c", "group"), ("d", "group")]),
                ),
                (USER, opts(&[("c", "user"), ("d", "user"), ("e", "user")])),
                (DEVICE_GROUP, opts(&[("d", "dg"), ("e", "dg"), ("f", "dg")])),
                (DEVICE, opts(&[("e", "device"), ("f", "device")])),
            ],
        );

        let effective = snap.effective(&inputs());

        let expected = opts(&[
            ("a", "global"),
            ("b", "group"),
            ("c", "user"),
            ("d", "dg"),
            ("e", "device"),
            ("f", "device"),
        ]);
        assert_eq!(effective.options, expected);
        let order: Vec<Target> = effective.applied.iter().map(|a| a.target).collect();
        let want = [
            Target::Global,
            Target::Group(7),
            Target::User(10),
            Target::DeviceGroup(30),
            Target::Device(20),
        ];
        assert_eq!(order, want);
    }

    #[test]
    fn device_group_outranks_the_owner_as_documented() {
        let snap = snapshot(
            &[
                at(Target::User(10), USER),
                at(Target::DeviceGroup(30), DEVICE_GROUP),
            ],
            &[
                (USER, opts(&[("k", "user")])),
                (DEVICE_GROUP, opts(&[("k", "dg")])),
            ],
        );
        assert_eq!(snap.effective(&inputs()).options, opts(&[("k", "dg")]));
    }

    #[test]
    fn empty_value_overrides_lower_layers_and_is_kept_for_the_client() {
        let snap = snapshot(
            &[at(Target::Global, GLOBAL), at(Target::Device(20), DEVICE)],
            &[
                (GLOBAL, opts(&[("keep", "v"), ("drop", "v")])),
                (DEVICE, opts(&[("drop", ""), ("never-set-below", "")])),
            ],
        );

        let effective = snap.effective(&inputs());

        assert_eq!(
            effective.options,
            opts(&[("keep", "v"), ("drop", ""), ("never-set-below", "")])
        );
        let live: Vec<_> = effective.live_options().collect();
        assert_eq!(live, [("keep", "v")]);
    }

    #[test]
    fn higher_non_empty_value_overrides_a_lower_empty_one() {
        let snap = snapshot(
            &[at(Target::Global, GLOBAL), at(Target::User(10), USER)],
            &[(GLOBAL, opts(&[("k", "")])), (USER, opts(&[("k", "back")]))],
        );
        assert_eq!(snap.effective(&inputs()).options, opts(&[("k", "back")]));
    }

    #[test]
    fn groups_tie_in_favour_of_the_higher_group_id() {
        let snap = snapshot(
            &[at(Target::Group(8), GROUP_B), at(Target::Group(7), GROUP_A)],
            &[
                (GROUP_A, opts(&[("k", "a"), ("only-a", "a")])),
                (GROUP_B, opts(&[("k", "b")])),
            ],
        );

        let effective = snap.effective(&inputs());

        assert_eq!(effective.options, opts(&[("k", "b"), ("only-a", "a")]));
        let order: Vec<Target> = effective.applied.iter().map(|a| a.target).collect();
        assert_eq!(order, [Target::Group(7), Target::Group(8)]);
    }

    #[test]
    fn same_strategy_on_several_layers_applies_at_each() {
        let snap = snapshot(
            &[at(Target::Global, GLOBAL), at(Target::Device(20), GLOBAL)],
            &[(GLOBAL, opts(&[("k", "v")]))],
        );
        let effective = snap.effective(&inputs());
        assert_eq!(effective.options, opts(&[("k", "v")]));
        assert_eq!(effective.applied.len(), 2);
    }

    #[test]
    fn layers_that_do_not_apply_are_ignored() {
        let snap = snapshot(
            &[
                at(Target::Group(7), GROUP_A),
                at(Target::User(99), USER),
                at(Target::DeviceGroup(99), DEVICE_GROUP),
                at(Target::Device(99), DEVICE),
            ],
            &[
                (GROUP_A, opts(&[("a", "1")])),
                (USER, opts(&[("u", "1")])),
                (DEVICE_GROUP, opts(&[("g", "1")])),
                (DEVICE, opts(&[("d", "1")])),
            ],
        );

        let ownerless = DeviceInputs {
            device_id: 20,
            ..DeviceInputs::default()
        };
        assert!(snap.effective(&ownerless).options.is_empty());
        let stranger = DeviceInputs {
            user_id: Some(12),
            ..inputs()
        };
        assert!(snap.effective(&stranger).options.is_empty());
        // Member of group 7 only through user 10.
        assert_eq!(snap.effective(&inputs()).options, opts(&[("a", "1")]));
    }

    #[test]
    fn assignment_to_an_unknown_strategy_is_skipped() {
        let snap = snapshot(&[at(Target::Global, 404)], &[]);
        let effective = snap.effective(&inputs());
        assert!(effective.options.is_empty() && effective.applied.is_empty());
    }

    #[test]
    fn modified_at_is_the_snapshot_version_or_a_later_device_change() {
        let snap = snapshot(&[], &[]);
        assert_eq!(snap.effective(&inputs()).modified_at, MODIFIED_AT);

        let moved = |ms: i64| DeviceInputs {
            changed_at: deskhaven_db::time::from_millis(ms),
            ..inputs()
        };
        assert_eq!(
            snap.effective(&moved(MODIFIED_AT + 5)).modified_at,
            MODIFIED_AT + 5
        );
        assert_eq!(
            snap.effective(&moved(MODIFIED_AT - 5)).modified_at,
            MODIFIED_AT
        );
    }
}
