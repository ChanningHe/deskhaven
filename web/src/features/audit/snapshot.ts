// Reading the before / after snapshots of console audit rows. They are stored redacted, so
// everything here is display logic: nothing is parsed out of them that was not already text.

export interface SnapshotChange {
  key: string
  kind: 'added' | 'removed' | 'changed'
  before: unknown
  after: unknown
}

export interface SnapshotDiff {
  changes: SnapshotChange[]
  /** Members present on both sides with the same value. */
  unchanged: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** An absent snapshot (a create has no `before`, a delete no `after`) is an empty object. */
function members(value: unknown): Record<string, unknown> | undefined {
  if (value === null || value === undefined) return {}
  return isRecord(value) ? value : undefined
}

/** The value as indented JSON text; the empty string for an absent one. */
export function showValue(value: unknown): string {
  if (value === undefined) return ''
  return JSON.stringify(value, null, 2)
}

function kindOf(key: string, before: Record<string, unknown>, after: Record<string, unknown>) {
  if (!(key in before)) return 'added'
  if (!(key in after)) return 'removed'
  return 'changed'
}

/**
 * The top-level members that differ between two snapshots, in the order they appear;
 * `undefined` when either side is not an object (then the snapshots are shown whole).
 */
export function diffSnapshots(beforeRaw: unknown, afterRaw: unknown): SnapshotDiff | undefined {
  const before = members(beforeRaw)
  const after = members(afterRaw)
  if (before === undefined || after === undefined) return undefined
  const keys = [...new Set([...Object.keys(before), ...Object.keys(after)])]
  const changes = keys
    .filter(
      (key) => key in before !== key in after || showValue(before[key]) !== showValue(after[key]),
    )
    .map((key): SnapshotChange => ({
      key,
      kind: kindOf(key, before, after),
      before: before[key],
      after: after[key],
    }))
  return { changes, unchanged: keys.length - changes.length }
}

export interface PurgeSummary {
  kind: string
  deleted: number
  cutoff: number
  batch: number
}

/** `console_audit.action` of a retention batch (04 §7). */
export const RETENTION_ACTION = 'audit.retention.purge'

/** The `{kind, deleted, cutoff, batch}` of a retention purge row, when `after` has that shape. */
export function purgeSummary(action: string, after: unknown): PurgeSummary | undefined {
  if (action !== RETENTION_ACTION || !isRecord(after)) return undefined
  const { kind, deleted, cutoff, batch } = after
  if (typeof kind !== 'string') return undefined
  if (typeof deleted !== 'number' || typeof cutoff !== 'number' || typeof batch !== 'number') {
    return undefined
  }
  return { kind, deleted, cutoff, batch }
}
