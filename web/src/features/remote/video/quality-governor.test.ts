import { describe, expect, it } from 'vitest'

import type { Quality } from '../settings/settings'
import { HOLD_TICKS, levelOf, QualityGovernor, START_QUALITY } from './quality-governor'

const RTT = 80
const calm = (n: number) => Array<number>(n).fill(RTT)

function governor(cap: Quality, warmUp: (number | null)[] = []) {
  const g = new QualityGovernor()
  g.setCap(cap)
  for (const delay of warmUp) g.tick(delay)
  return g
}

const run = (g: QualityGovernor, delays: (number | null)[]) => delays.map((d) => g.tick(d))

function backedOff() {
  const g = governor('balanced', [...calm(3), RTT + 250])
  expect(g.current).toEqual({ custom: 16 })
  return g
}

describe('QualityGovernor', () => {
  it.each<[string, Quality, (number | null)[], (Quality | null)[]]>([
    ['never moves without a delay', 'best', [null, null, null], [null, null, null]],
    [
      'climbs geometrically and sends the preset itself at the top',
      'balanced',
      calm(4),
      [{ custom: 18 }, { custom: 27 }, 'balanced', null],
    ],
    [
      'climbs to best in five calm ticks',
      'best',
      calm(5),
      [{ custom: 18 }, { custom: 27 }, { custom: 41 }, { custom: 62 }, 'best'],
    ],
    [
      'stops at a custom cap',
      { custom: 30 },
      calm(3),
      [{ custom: 18 }, { custom: 27 }, { custom: 30 }],
    ],
    [
      'holds while the delay is elevated but not congested',
      'best',
      [RTT, RTT + 150, RTT + 101, RTT + 100],
      [{ custom: 18 }, null, null, { custom: 27 }],
    ],
    [
      'measures the excess from the lowest delay seen',
      'best',
      [300, RTT, 250, 250 + 1],
      [{ custom: 18 }, { custom: 27 }, null, null],
    ],
    [
      'neither climbs nor backs off on a tick without a fresh delay',
      'best',
      [RTT, null, null, RTT],
      [{ custom: 18 }, null, null, { custom: 27 }],
    ],
    [
      'backs off while the next delay is overdue, as for a climbing one',
      'best',
      [RTT, RTT, null, null, null, null, null, null, null],
      [{ custom: 18 }, { custom: 27 }, null, null, { custom: 13 }, null, null, null, START_QUALITY],
    ],
    [
      'never backs off below the start',
      'best',
      [RTT, 2000, 3000, 4000],
      [{ custom: 18 }, START_QUALITY, null, null],
    ],
  ])('%s', (_, cap, delays, sent) => {
    const g = governor(cap)
    expect(run(g, delays)).toEqual(sent)
  })

  it('starts from the quality the login sent', () => {
    const g = new QualityGovernor()
    expect(g.current).toEqual(START_QUALITY)
    expect(g.setCap(START_QUALITY)).toBeNull()
    expect(run(g, calm(3))).toEqual([null, null, null])
  })

  it('halves on a backlog, then waits for the queue to drain', () => {
    const g = governor('balanced', calm(3))
    expect(g.current).toBe('balanced')
    expect(run(g, [RTT + 250, RTT + 900, RTT + 1500, RTT + 1200, RTT + 800, RTT + 300])).toEqual([
      { custom: 16 },
      null,
      null,
      null,
      null,
      null,
    ])
  })

  it('halves again once the cool-down is over and the delay still climbs', () => {
    const g = backedOff()
    expect(run(g, [RTT + 400, RTT + 600, RTT + 800, RTT + 900])).toEqual([
      null,
      null,
      null,
      { custom: 12 },
    ])
  })

  it('backs off in the middle of a session, not only at its start', () => {
    const g = governor('best', calm(60))
    expect(g.current).toBe('best')
    expect(g.tick(RTT + 201)).toEqual({ custom: 37 })
  })

  it('recovers after a calm hold: geometric to the threshold, then a level per hold', () => {
    const g = backedOff()
    expect(run(g, calm(HOLD_TICKS - 1))).toEqual([null, null])
    expect(run(g, calm(10))).toEqual([
      { custom: 24 },
      { custom: 28 },
      null,
      { custom: 29 },
      null,
      null,
      { custom: 30 },
      null,
      null,
      { custom: 31 },
    ])
  })

  it('starts the hold over when the delay rises during it', () => {
    const g = backedOff()
    expect(run(g, [RTT, RTT, RTT + 150, RTT, RTT, RTT])).toEqual([
      null,
      null,
      null,
      null,
      null,
      { custom: 24 },
    ])
  })

  it.each<[string, Quality, Quality | null]>([
    ['lowers to a lower preset at once', 'low', 'low'],
    ['lowers to a lower custom quality at once', { custom: 20 }, { custom: 20 }],
    ['sends the new choice when it is the same level', { custom: 33 }, { custom: 33 }],
    ['waits for calm ticks to rise to a higher cap', 'best', null],
  ])('%s', (_, cap, sent) => {
    const g = governor('balanced', calm(3))
    expect(g.setCap(cap)).toEqual(sent)
  })

  it('climbs on toward a raised cap', () => {
    const g = governor('balanced', calm(3))
    g.setCap('best')
    expect(run(g, calm(2))).toEqual([{ custom: 50 }, 'best'])
  })

  it('takes a cap below the start as its floor', () => {
    const g = new QualityGovernor()
    expect(g.setCap({ custom: 10 })).toEqual({ custom: 10 })
    expect(run(g, [RTT, RTT + 500, RTT + 900, RTT, RTT])).toEqual([null, null, null, null, null])
  })

  it('keeps a lowered cap through a later back-off and recovery', () => {
    const g = governor('best', calm(5))
    expect(g.setCap('low')).toBe('low')
    expect(g.tick(RTT + 300)).toEqual(START_QUALITY)
    expect(run(g, calm(HOLD_TICKS + 3))).toEqual([
      null,
      null,
      { custom: 18 },
      { custom: 21 },
      null,
      { custom: 22 },
    ])
  })

  describe('above level 100', () => {
    const climb = [18, 27, 41, 62, 93, 118, 143, 168, 193, 218, 243, 268, 293, 318, 343, 368]
    const high = [393, 418, 443, 468, 493]

    it('climbs at most 25 levels a tick, up to the cap', () => {
      const g = governor({ custom: 500 })
      const sent = run(g, calm(climb.length + high.length + 2))
      expect(sent).toEqual([...[...climb, ...high, 500].map((custom) => ({ custom })), null])
    })

    it('stops at a custom cap between 100 and 500', () => {
      const g = governor({ custom: 200 }, calm(8))
      expect(g.current).toEqual({ custom: 168 })
      expect(run(g, calm(3))).toEqual([{ custom: 193 }, { custom: 200 }, null])
    })

    it('halves on a backlog, then recovers 25 levels a tick to the threshold', () => {
      const g = governor({ custom: 500 }, calm(30))
      expect(g.current).toEqual({ custom: 500 })
      expect(g.tick(RTT + 250)).toEqual({ custom: 250 })
      expect(run(g, calm(HOLD_TICKS - 1))).toEqual([null, null])
      expect(run(g, calm(7)).map((q) => q && levelOf(q))).toEqual([
        275, 300, 325, 350, 375, 400, 425,
      ])
    })

    it('recovers past the threshold in steps scaled to the level', () => {
      const g = governor({ custom: 500 }, calm(30))
      g.tick(RTT + 250)
      run(g, calm(HOLD_TICKS - 1 + 7))
      expect(g.current).toEqual({ custom: 425 })
      expect(run(g, calm(6))).toEqual([null, null, { custom: 429 }, null, null, { custom: 433 }])
    })

    it('lowers to a cap below the level at once', () => {
      const g = governor({ custom: 500 }, calm(30))
      expect(g.setCap({ custom: 150 })).toEqual({ custom: 150 })
      expect(g.setCap('best')).toBe('best')
    })
  })
})
