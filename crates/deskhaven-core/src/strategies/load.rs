//! Reads a [`Snapshot`] from the database: three plain queries, no transaction. A writer
//! committing in between can make the snapshot mixed, but its own invalidation (after
//! commit) bumps the cache generation, so such a snapshot is never kept.

use std::collections::{HashMap, HashSet};

use deskhaven_db::entity::{group_member, strategy, strategy_assignment};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

use super::StrategyError;
use super::effective::Snapshot;
use super::model::{Assignment, Target};
use super::options::{self, Options};

pub(super) async fn snapshot(
    db: &impl ConnectionTrait,
    version: i64,
) -> Result<Snapshot, StrategyError> {
    let assignments = assignments(db).await?;
    let options = assigned_options(db, &assignments).await?;
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
    assignments: &[Assignment],
) -> Result<HashMap<i64, Options>, StrategyError> {
    let assigned: HashSet<i64> = assignments.iter().map(|a| a.strategy_id).collect();
    let strategies = strategy::Entity::find().all(db).await?;
    let mut options = HashMap::with_capacity(assigned.len());
    for row in strategies.into_iter().filter(|s| assigned.contains(&s.id)) {
        let parsed = options::decode(&row.options).ok_or(StrategyError::Corrupt {
            strategy_id: row.id,
        })?;
        options.insert(row.id, parsed);
    }
    Ok(options)
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
