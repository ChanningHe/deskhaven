//! Reads a [`Snapshot`] from the database: three plain queries, deliberately not in one
//! read transaction. A writer committing in between can make the snapshot mixed, but every
//! writer invalidates the cache after its commit (in a task the caller cannot cancel), which
//! bumps the generation, so such a snapshot is dropped by `StrategyCache::store` or by that
//! invalidation, and it is served at most for the moment before the invalidation runs.

use std::collections::{HashMap, HashSet};

use deskhaven_db::entity::{group_member, strategy, strategy_assignment};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Select};

use super::StrategyError;
use super::effective::Snapshot;
use super::model::{Assignment, Target};
use super::options::Options;
use super::sealing;
use crate::crypto::SecretBox;

pub(super) async fn snapshot(
    db: &impl ConnectionTrait,
    crypto: &SecretBox,
    version: i64,
) -> Result<Snapshot, StrategyError> {
    let assignments = assignments(db).await?;
    let options = assigned_options(db, crypto, &assignments).await?;
    let members = group_members(db, &assignments).await?;
    Ok(Snapshot::build(version, &assignments, options, members))
}

/// Every assignment. A row that breaks the schema's `target_id` rule (impossible while the
/// CHECK exists) is skipped with a warning.
pub(super) async fn assignments(
    db: &impl ConnectionTrait,
) -> Result<Vec<Assignment>, StrategyError> {
    let rows = strategy_assignment::Entity::find().all(db).await?;
    let mut assignments = Vec::with_capacity(rows.len());
    for row in rows {
        match Target::from_row(row.target_type, row.target_id) {
            Some(target) => assignments.push(Assignment {
                target,
                strategy_id: row.strategy_id,
            }),
            None => tracing::warn!(
                assignment_id = row.id,
                "skipping malformed strategy assignment"
            ),
        }
    }
    Ok(assignments)
}

/// Options of every strategy that has an assignment.
async fn assigned_options(
    db: &impl ConnectionTrait,
    crypto: &SecretBox,
    assignments: &[Assignment],
) -> Result<HashMap<i64, Options>, StrategyError> {
    let assigned: HashSet<i64> = assignments.iter().map(|a| a.strategy_id).collect();
    if assigned.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = strategies_by_id(&assigned).all(db).await?;
    let mut options = HashMap::with_capacity(rows.len());
    for row in rows {
        // One damaged row must not take every device's strategy down with it: its layer is
        // skipped, and the console shows it flagged (`Strategy::options_corrupt`).
        match sealing::decode(crypto, row.id, &row.options) {
            Some(opened) => drop(options.insert(row.id, opened.options)),
            None => tracing::error!(
                strategy_id = row.id,
                "stored strategy options are unreadable; the strategy is ignored"
            ),
        }
    }
    Ok(options)
}

/// Only the assigned strategies: options are up to ~1 MB each and most strategies may be
/// unassigned.
fn strategies_by_id(ids: &HashSet<i64>) -> Select<strategy::Entity> {
    strategy::Entity::find().filter(strategy::Column::Id.is_in(ids.iter().copied()))
}

/// Members of the user groups that have an assignment.
async fn group_members(
    db: &impl ConnectionTrait,
    assignments: &[Assignment],
) -> Result<HashMap<i64, HashSet<i64>>, StrategyError> {
    let groups: Vec<i64> = assignments
        .iter()
        .filter_map(|a| match a.target {
            Target::Group(id) => Some(id),
            _ => None,
        })
        .collect();
    let mut members: HashMap<i64, HashSet<i64>> = HashMap::new();
    if groups.is_empty() {
        return Ok(members);
    }
    let rows = group_member::Entity::find()
        .filter(group_member::Column::GroupId.is_in(groups))
        .all(db)
        .await?;
    for row in rows {
        members.entry(row.group_id).or_default().insert(row.user_id);
    }
    Ok(members)
}

#[cfg(test)]
mod tests {
    use sea_orm::{DbBackend, QueryTrait};

    use super::*;

    #[test]
    fn only_assigned_strategies_are_selected() {
        let ids = HashSet::from([4]);
        for backend in [DbBackend::Postgres, DbBackend::Sqlite] {
            let sql = strategies_by_id(&ids).build(backend).to_string();

            assert!(sql.contains("\"id\" IN (4)"), "{backend:?}: {sql}");
        }
    }
}
