import { describe, expect, it } from 'vitest'

import { versionBars } from './bars'

describe('versionBars', () => {
  it('scales the bars to the largest count and appends other and unknown', () => {
    const bars = versionBars({
      versions: [
        { version: '1.4.1', count: 8 },
        { version: '1.3.0', count: 2 },
      ],
      other: 4,
      unknown: 1,
    })

    expect(bars.map((b) => [b.kind, b.count, b.width])).toEqual([
      ['version', 8, 100],
      ['version', 2, 25],
      ['other', 4, 50],
      ['unknown', 1, 12.5],
    ])
  })

  it('leaves out empty other and unknown bars and survives an empty fleet', () => {
    expect(versionBars({ versions: [], other: 0, unknown: 0 })).toEqual([])
    const only = versionBars({ versions: [{ version: '1.4.1', count: 3 }], other: 0, unknown: 0 })
    expect(only.map((b) => b.kind)).toEqual(['version'])
  })
})
