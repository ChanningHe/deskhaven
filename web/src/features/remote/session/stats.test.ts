import { describe, expect, it } from 'vitest'

import { FakeDecoders, fixtureKeyFrame, vp9Frame } from '../video/fake-decoder'
import { VideoPipeline } from '../video/pipeline'
import { SessionMeter, STATS_INTERVAL_MS } from './stats'
import { ValueStore } from './store'
import { FakeClock } from './testing'

function setup() {
  const clock = new FakeClock()
  const state = new ValueStore<{ kind: string }>({ kind: 'connecting' })
  const decoders = new FakeDecoders()
  let pipeline: VideoPipeline | null = null
  const meter = new SessionMeter(state, clock, () => ({
    decodeQueue: decoders.created.at(-1)?.decodeQueueSize ?? 0,
    codec: pipeline?.codec ?? null,
    width: 1280,
    height: 800,
  }))
  pipeline = new VideoPipeline({
    createDecoder: decoders.create,
    onFrame: (frame) => {
      frame.close()
    },
    requestKeyFrame: () => {
      meter.keyRequested()
    },
    mayRefresh: () => true,
    onFatal: () => undefined,
    meter,
  })
  return { clock, state, decoders, meter, pipeline }
}

describe('SessionMeter', () => {
  it('does not tick before the session is active', () => {
    const { clock, meter } = setup()
    expect(clock.timers).toBe(0)
    clock.tick()
    expect(meter.stats.get()).toMatchObject({ fps: 0, kbps: 0, delayMs: null })
  })

  it('rolls its per-second figures over once a second while active', () => {
    const { clock, state, decoders, meter, pipeline } = setup()
    state.set({ kind: 'active' })
    expect(clock.timers).toBe(1)

    const key = fixtureKeyFrame()
    pipeline.push(key)
    clock.advance(10)
    decoders.last.output()
    pipeline.push(vp9Frame(false, new Uint8Array(456)))
    clock.advance(30)
    decoders.last.output()
    meter.pointerMoved()
    clock.tick(STATS_INTERVAL_MS - 40)

    expect(meter.stats.get()).toEqual({
      fps: 2,
      kbps: Math.round(((544 + 456) * 8) / STATS_INTERVAL_MS),
      delayMs: null,
      decodeQueue: 0,
      decodeMs: 20,
      keyRequests: 0,
      pointerMoves: 1,
      width: 1280,
      height: 800,
      codec: 'vp09.00.10.08',
    })

    decoders.last.decodeQueueSize = 3
    clock.tick()
    expect(meter.stats.get()).toMatchObject({
      fps: 0,
      kbps: 0,
      decodeMs: null,
      pointerMoves: 0,
      decodeQueue: 3,
      codec: 'vp09.00.10.08',
    })
  })

  it('scales the per-second figures to the time that really passed', () => {
    const { clock, state, meter } = setup()
    state.set({ kind: 'active' })
    for (let i = 0; i < 3; i++) meter.pointerMoved()
    clock.tick(2 * STATS_INTERVAL_MS)
    expect(meter.stats.get().pointerMoves).toBe(1.5)
  })

  it('has no delay until the peer reports one, then keeps the last', () => {
    const { clock, state, meter } = setup()
    state.set({ kind: 'active' })
    clock.tick()
    expect(meter.stats.get().delayMs).toBeNull()
    meter.delay(42)
    meter.delay(37)
    clock.tick()
    clock.tick()
    expect(meter.stats.get().delayMs).toBe(37)
  })

  it('counts key frame requests over the whole session', () => {
    const { clock, state, meter, pipeline } = setup()
    state.set({ kind: 'active' })
    pipeline.push(vp9Frame(false))
    pipeline.push(vp9Frame(false))
    clock.tick()
    expect(meter.stats.get().keyRequests).toBe(1)

    pipeline.push(fixtureKeyFrame())
    pipeline.reset()
    pipeline.push(vp9Frame(false))
    clock.tick()
    clock.tick()
    expect(meter.stats.get().keyRequests).toBe(2)
  })

  it('stops for good once the session is closed', () => {
    const { clock, state, meter } = setup()
    state.set({ kind: 'active' })
    meter.pointerMoved()
    clock.tick()
    const last = meter.stats.get()

    state.set({ kind: 'closed' })
    expect(clock.timers).toBe(0)
    state.set({ kind: 'active' })
    expect(clock.timers).toBe(0)
    clock.tick()
    expect(meter.stats.get()).toBe(last)
  })

  it('keeps ticking through a later step that is not closed', () => {
    const { clock, state } = setup()
    state.set({ kind: 'active' })
    state.set({ kind: 'connecting' })
    state.set({ kind: 'active' })
    expect(clock.timers).toBe(1)
  })
})
