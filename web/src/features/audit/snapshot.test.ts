import { describe, expect, it } from 'vitest'

import { diffSnapshots, purgeSummary, showValue } from './snapshot'

describe('snapshot diff', () => {
  it('lists added, removed and changed members and counts the rest', () => {
    const diff = diffSnapshots(
      { role: 'user', note: 'x', same: 1 },
      { role: 'admin', extra: true, same: 1 },
    )

    expect(diff?.changes).toEqual([
      { key: 'role', kind: 'changed', before: 'user', after: 'admin' },
      { key: 'note', kind: 'removed', before: 'x', after: undefined },
      { key: 'extra', kind: 'added', before: undefined, after: true },
    ])
    expect(diff?.unchanged).toBe(1)
  })

  it('treats an absent side as empty, so a create shows every member as added', () => {
    const diff = diffSnapshots(null, { name: 'ci' })

    expect(diff?.changes.map((c) => [c.key, c.kind])).toEqual([['name', 'added']])
    expect(diffSnapshots({ a: 1 }, undefined)?.changes[0]?.kind).toBe('removed')
  })

  it('tells a member set to null from a missing one', () => {
    const diff = diffSnapshots({ a: null }, {})

    expect(diff?.changes).toEqual([{ key: 'a', kind: 'removed', before: null, after: undefined }])
  })

  it('compares nested values by content', () => {
    const diff = diffSnapshots({ tags: ['a', 'b'] }, { tags: ['a', 'b'] })

    expect(diff).toEqual({ changes: [], unchanged: 1 })
  })

  it('gives up on snapshots that are not objects', () => {
    expect(diffSnapshots([1], { a: 1 })).toBeUndefined()
    expect(diffSnapshots('text', null)).toBeUndefined()
  })

  it('shows values as indented JSON', () => {
    expect(showValue({ a: 1 })).toBe('{\n  "a": 1\n}')
    expect(showValue(undefined)).toBe('')
    expect(showValue(null)).toBe('null')
  })
})

describe('retention purge summary', () => {
  const after = { kind: 'conn', deleted: 12, cutoff: 1_700_000_000_000, batch: 500 }

  it('reads the purge row written by the retention job', () => {
    expect(purgeSummary('audit.retention.purge', after)).toEqual(after)
  })

  it('ignores other actions and malformed snapshots', () => {
    expect(purgeSummary('user.update', after)).toBeUndefined()
    expect(purgeSummary('audit.retention.purge', { ...after, deleted: '12' })).toBeUndefined()
    expect(purgeSummary('audit.retention.purge', null)).toBeUndefined()
    expect(purgeSummary('audit.retention.purge', [1])).toBeUndefined()
  })
})
