import { describe, expect, it } from 'vitest'

import {
  DEFAULT_FPS,
  FpsGovernor,
  INACTIVE_TICKS,
  MAX_REFRESHES,
  REFRESH_INTERVAL_MS,
} from './fps-governor'

/** The first three decodes are skipped as warm-up; ten more give the first rate. */
function governor(serviceMs: number, direct = false, userFps = DEFAULT_FPS): FpsGovernor {
  const g = new FpsGovernor({ direct, userFps })
  for (let i = 0; i < 13; i++) g.decoded(serviceMs)
  return g
}

describe('FpsGovernor fps', () => {
  it('reports nothing until it knows how fast frames decode', () => {
    const g = new FpsGovernor({ direct: false, userFps: DEFAULT_FPS })
    for (let i = 0; i < 12; i++) g.decoded(10)
    expect(g.tick({ fps: 20, queued: 0 })).toBeNull()
    g.decoded(10)
    expect(g.tick({ fps: 20, queued: 0 })).toBe(DEFAULT_FPS)
  })

  it('leaves out the warm-up decodes', () => {
    const g = new FpsGovernor({ direct: false, userFps: DEFAULT_FPS })
    for (let i = 0; i < 3; i++) g.decoded(1000)
    for (let i = 0; i < 10; i++) g.decoded(50)
    expect(g.tick({ fps: 20, queued: 0 })).toBe(16)
  })

  it.each([
    [false, 16],
    [true, 18],
  ])('keeps a margin below the decode rate (direct %s: 20 fps decoded → %i)', (direct, fps) => {
    expect(governor(50, direct).tick({ fps: 20, queued: 0 })).toBe(fps)
  })

  it('never asks for more than the user fps', () => {
    expect(governor(1).tick({ fps: 20, queued: 0 })).toBe(DEFAULT_FPS)
    expect(governor(1, false, 12).tick({ fps: 20, queued: 0 })).toBe(12)
  })

  it('reports a value only when it changes', () => {
    const g = governor(50)
    expect(g.tick({ fps: 16, queued: 0 })).toBe(16)
    for (let i = 0; i < 10; i++) expect(g.tick({ fps: 16, queued: 0 })).toBeNull()
  })

  it('halves the fps when the decode backlog is longer than the fps it can keep', () => {
    const g = governor(50)
    expect(g.tick({ fps: 16, queued: 0 })).toBe(16)
    expect(g.tick({ fps: 16, queued: 11 })).toBeNull()
    expect(g.tick({ fps: 16, queued: 17 })).toBe(8)
  })

  it('climbs back once the backlog has stayed empty for a while', () => {
    const g = governor(50)
    g.tick({ fps: 16, queued: 0 })
    expect(g.tick({ fps: 16, queued: 17 })).toBe(8)
    for (let i = 0; i < 3; i++) expect(g.tick({ fps: 8, queued: 1 })).toBeNull()
    expect(g.tick({ fps: 8, queued: 0 })).toBe(16)
  })

  it('stops steering a screen that has not changed for a while', () => {
    const g = governor(50)
    g.tick({ fps: 16, queued: 0 })
    for (let i = 0; i < INACTIVE_TICKS; i++) g.tick({ fps: 0, queued: 0 })
    expect(g.tick({ fps: 0, queued: 17 })).toBeNull()
    expect(g.tick({ fps: 1, queued: 17 })).toBe(8)
  })
})

describe('FpsGovernor user ceiling', () => {
  it('brings the fps down on the next tick when the ceiling drops below it', () => {
    const g = governor(1)
    expect(g.tick({ fps: 20, queued: 0 })).toBe(DEFAULT_FPS)
    g.setUserFps(12)
    expect(g.tick({ fps: 20, queued: 0 })).toBe(12)
    expect(g.tick({ fps: 20, queued: 0 })).toBeNull()
  })

  it('lets the fps climb to a higher ceiling the way it climbs after a backlog', () => {
    const g = governor(1)
    expect(g.tick({ fps: 20, queued: 0 })).toBe(DEFAULT_FPS)
    g.setUserFps(60)
    const sent = Array.from({ length: 6 }, () => g.tick({ fps: 20, queued: 0 }))
    expect(sent.filter((fps) => fps !== null)).toEqual([60])
    expect(sent.indexOf(60)).toBeGreaterThan(0)
  })

  it('does not climb past what the decoder keeps up with', () => {
    const g = governor(50, true)
    expect(g.tick({ fps: 18, queued: 0 })).toBe(18)
    g.setUserFps(60)
    for (let i = 0; i < 10; i++) expect(g.tick({ fps: 18, queued: 0 })).toBeNull()
  })

  it('sends nothing by itself', () => {
    const g = new FpsGovernor({ direct: false, userFps: DEFAULT_FPS })
    g.setUserFps(12)
    expect(g.tick({ fps: 20, queued: 0 })).toBeNull()
  })
})

describe('FpsGovernor refresh', () => {
  it(`allows a refresh at once, then one per ${REFRESH_INTERVAL_MS} ms`, () => {
    const g = governor(50)
    expect(g.mayRefresh(5_000)).toBe(true)
    expect(g.mayRefresh(5_000 + REFRESH_INTERVAL_MS)).toBe(false)
    expect(g.mayRefresh(5_001 + REFRESH_INTERVAL_MS)).toBe(true)
  })

  it(`allows at most ${MAX_REFRESHES} refreshes`, () => {
    const g = governor(50)
    let now = 0
    let allowed = 0
    for (let i = 0; i < MAX_REFRESHES + 5; i++) {
      if (g.mayRefresh(now)) allowed++
      now += REFRESH_INTERVAL_MS + 1
    }
    expect(allowed).toBe(MAX_REFRESHES)
  })
})
