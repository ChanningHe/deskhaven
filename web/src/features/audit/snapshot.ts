// Snapshots are stored redacted: everything here is display logic.

export interface SnapshotChange {
  key: string
  kind: 'added' | 'removed' | 'changed'
  before: unknown
  after: unknown
}

export interface SnapshotDiff {
  changes: SnapshotChange[]
  unchanged: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function members(value: unknown): Record<string, unknown> | undefined {
  if (value === null || value === undefined) return {}
  return isRecord(value) ? value : undefined
}

export function showValue(value: unknown): string {
  if (value === undefined) return ''
  return JSON.stringify(value, null, 2)
}

function kindOf(key: string, before: Record<string, unknown>, after: Record<string, unknown>) {
  if (!(key in before)) return 'added'
  if (!(key in after)) return 'removed'
  return 'changed'
}

/** `undefined` when either side is not an object (then the snapshots are shown whole). */
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

export const RETENTION_ACTION = 'audit.retention.purge'

export function purgeSummary(action: string, after: unknown): PurgeSummary | undefined {
  if (action !== RETENTION_ACTION || !isRecord(after)) return undefined
  const { kind, deleted, cutoff, batch } = after
  if (typeof kind !== 'string') return undefined
  if (typeof deleted !== 'number' || typeof cutoff !== 'number' || typeof batch !== 'number') {
    return undefined
  }
  return { kind, deleted, cutoff, batch }
}
