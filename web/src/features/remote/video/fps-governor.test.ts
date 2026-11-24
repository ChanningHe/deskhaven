import { describe, expect, it } from 'vitest'

import {
  BACKLOG,
  FpsGovernor,
  INACTIVE_TICKS,
  MAX_REFRESHES,
  MIN_AUTO_FPS,
  REFRESH_INTERVAL_MS,
} from './fps-governor'

const KEEPING_UP = { fps: 60, backlog: 0 }
const BACKED_UP = { fps: 60, backlog: BACKLOG }

/** Ticks until the governor has nothing more to send; returns what it sent. */
function settle(g: FpsGovernor, tick = KEEPING_UP): number[] {
  const sent: number[] = []
  for (let i = 0; i < 100; i++) {
    const fps = g.tick(tick)
    if (fps !== null) sent.push(fps)
  }
  return sent
}

describe('FpsGovernor fps', () => {
  it('stays at the fps it logged in with while the decoder keeps up', () => {
    const g = new FpsGovernor({ userFps: 60 })
    expect(settle(g)).toEqual([])
  })

  it('is not held back by a slow decode while nothing backs up', () => {
    const g = new FpsGovernor({ userFps: 60 })
    for (let i = 0; i < 13; i++) g.decoded(50)
    expect(g.decodeFps).toBe(20)
    expect(settle(g, { fps: 60, backlog: BACKLOG - 1 })).toEqual([])
  })

  it('skips the first decodes of a new decoder, as it does those of the first one', () => {
    const g = new FpsGovernor({ userFps: 60 })
    for (let i = 0; i < 13; i++) g.decoded(50)
    expect(g.decodeFps).toBe(20)
    g.decoderStarted()
    for (let i = 0; i < 3; i++) g.decoded(1000)
    for (let i = 0; i < 10; i++) g.decoded(50)
    expect(g.decodeFps).toBe(20)
  })

  it('halves the fps while the decodes back up, down to a floor', () => {
    const g = new FpsGovernor({ userFps: 60 })
    expect(settle(g, BACKED_UP)).toEqual([30, 15, 7, MIN_AUTO_FPS])
  })

  it('halves once over three backed-up ticks, and again if the backlog outlasts the cut', () => {
    const g = new FpsGovernor({ userFps: 60 })
    expect(g.tick(BACKED_UP)).toBe(30)
    expect(g.tick(BACKED_UP)).toBeNull()
    expect(g.tick(BACKED_UP)).toBeNull()
    expect(g.tick(BACKED_UP)).toBe(15)
  })

  it('ignores the backlog of a decoder that has just started, then halves on it', () => {
    const g = new FpsGovernor({ userFps: 60 })
    g.decoderStarted()
    expect(g.tick({ fps: 60, backlog: 4 })).toBeNull()
    expect(g.tick({ fps: 60, backlog: 4 })).toBeNull()
    expect(g.tick({ fps: 60, backlog: 4 })).toBe(30)
  })

  it('climbs back to the user fps once the backlog has stayed away for a while', () => {
    const g = new FpsGovernor({ userFps: 60 })
    expect(g.tick(BACKED_UP)).toBe(30)
    for (let i = 0; i < 3; i++) expect(g.tick(KEEPING_UP)).toBeNull()
    expect(g.tick(KEEPING_UP)).toBe(33)
    expect(settle(g).at(-1)).toBe(60)
  })

  it('starts waiting again when the decodes back up on the way', () => {
    const g = new FpsGovernor({ userFps: 60 })
    g.tick(BACKED_UP)
    for (let i = 0; i < 3; i++) g.tick(KEEPING_UP)
    expect(g.tick(KEEPING_UP)).toBe(33)
    expect(g.tick(BACKED_UP)).toBe(16)
    for (let i = 0; i < 3; i++) expect(g.tick(KEEPING_UP)).toBeNull()
    expect(g.tick(KEEPING_UP)).toBe(19)
  })

  it('ignores the backlog of a screen that has not changed for a while', () => {
    const g = new FpsGovernor({ userFps: 60 })
    for (let i = 0; i < INACTIVE_TICKS; i++) g.tick({ fps: 0, backlog: 0 })
    expect(g.tick({ fps: 0, backlog: BACKLOG + 10 })).toBeNull()
    expect(g.tick({ fps: 1, backlog: BACKLOG + 10 })).toBe(30)
  })

  it('takes a backlog with no frames coming out as one, before the screen counts as still', () => {
    const g = new FpsGovernor({ userFps: 60 })
    expect(g.tick({ fps: 0, backlog: BACKLOG })).toBe(30)
  })
})

describe('FpsGovernor user ceiling', () => {
  it('lowers to a lower user fps on the next tick', () => {
    const g = new FpsGovernor({ userFps: 60 })
    g.setUserFps(12)
    expect(g.tick(KEEPING_UP)).toBe(12)
    expect(settle(g)).toEqual([])
  })

  it('raises to a higher user fps on the next tick, before any frame was decoded', () => {
    const g = new FpsGovernor({ userFps: 30 })
    g.setUserFps(60)
    expect(g.tick({ fps: 0, backlog: 0 })).toBe(60)
    expect(settle(g)).toEqual([])
  })

  it('raises to a higher user fps on the next tick, even while it waits out a backlog', () => {
    const g = new FpsGovernor({ userFps: 60 })
    expect(g.tick(BACKED_UP)).toBe(30)
    g.setUserFps(50)
    expect(g.tick(KEEPING_UP)).toBe(50)
  })

  it('follows the user on a screen that has not changed for a while', () => {
    const g = new FpsGovernor({ userFps: 30 })
    for (let i = 0; i < INACTIVE_TICKS; i++) g.tick({ fps: 0, backlog: 0 })
    g.setUserFps(10)
    expect(g.tick({ fps: 0, backlog: 0 })).toBe(10)
    g.setUserFps(60)
    expect(g.tick({ fps: 0, backlog: 0 })).toBe(60)
  })

  it('sends nothing for a ceiling that did not change', () => {
    const g = new FpsGovernor({ userFps: 30 })
    g.setUserFps(30)
    expect(g.tick(KEEPING_UP)).toBeNull()
  })

  it('never climbs past the user fps', () => {
    const g = new FpsGovernor({ userFps: 20 })
    g.tick(BACKED_UP)
    expect(Math.max(...settle(g))).toBe(20)
  })
})

describe('FpsGovernor decode estimate', () => {
  it('has none until ten decodes after the warm-up', () => {
    const g = new FpsGovernor({ userFps: 30 })
    for (let i = 0; i < 12; i++) g.decoded(10)
    expect(g.decodeFps).toBeNull()
    g.decoded(10)
    expect(g.decodeFps).toBe(100)
  })

  it('leaves out the warm-up decodes', () => {
    const g = new FpsGovernor({ userFps: 30 })
    for (let i = 0; i < 3; i++) g.decoded(1000)
    for (let i = 0; i < 10; i++) g.decoded(50)
    expect(g.decodeFps).toBe(20)
  })
})

describe('FpsGovernor refresh', () => {
  it(`allows a refresh at once, then one per ${REFRESH_INTERVAL_MS} ms`, () => {
    const g = new FpsGovernor({ userFps: 30 })
    expect(g.mayRefresh(5_000)).toBe(true)
    expect(g.mayRefresh(5_000 + REFRESH_INTERVAL_MS)).toBe(false)
    expect(g.mayRefresh(5_001 + REFRESH_INTERVAL_MS)).toBe(true)
  })

  it(`allows at most ${MAX_REFRESHES} refreshes`, () => {
    const g = new FpsGovernor({ userFps: 30 })
    let now = 0
    let allowed = 0
    for (let i = 0; i < MAX_REFRESHES + 5; i++) {
      if (g.mayRefresh(now)) allowed++
      now += REFRESH_INTERVAL_MS + 1
    }
    expect(allowed).toBe(MAX_REFRESHES)
  })
})
