//! The pending connections of one port, for eviction at its budget (M4-05).

use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;

/// The pending connections of one port, grouped by source, oldest first per group.
#[derive(Default)]
pub(super) struct PendingSet {
    by_group: HashMap<IpAddr, BTreeSet<u64>>,
    /// One key per group; the last is the group holding the most, the one whose oldest
    /// connection is the oldest among equals.
    heaviest: BTreeSet<GroupKey>,
    len: usize,
}

/// (pending count, reversed id of the group's oldest connection, group).
type GroupKey = (usize, Reverse<u64>, IpAddr);

fn group_key(ids: &BTreeSet<u64>, group: IpAddr) -> Option<GroupKey> {
    Some((ids.len(), Reverse(*ids.first()?), group))
}

impl PendingSet {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// Applies `change` to `group`'s ids, keeping the ordering index in step.
    fn update(&mut self, group: IpAddr, change: impl FnOnce(&mut BTreeSet<u64>) -> bool) {
        let ids = self.by_group.entry(group).or_default();
        if let Some(key) = group_key(ids, group) {
            self.heaviest.remove(&key);
        }
        let before = ids.len();
        change(ids);
        self.len = self.len - before + ids.len();
        match group_key(ids, group) {
            Some(key) => {
                self.heaviest.insert(key);
            }
            None => {
                self.by_group.remove(&group);
            }
        }
    }

    pub(super) fn add(&mut self, group: IpAddr, id: u64) {
        self.update(group, |ids| ids.insert(id));
    }

    pub(super) fn remove(&mut self, group: IpAddr, id: u64) {
        if self.by_group.contains_key(&group) {
            self.update(group, |ids| ids.remove(&id));
        }
    }

    /// The oldest pending connection of the group holding the most.
    pub(super) fn victim(&self) -> Option<u64> {
        let (_, Reverse(oldest), _) = self.heaviest.last()?;
        Some(*oldest)
    }
}
