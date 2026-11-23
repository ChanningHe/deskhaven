import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import { NO_MODIFIERS } from '../input/mouse'
import { TestDelaySchema } from '../proto/gen/message_pb.js'
import { vp9Frame } from '../video/fake-decoder'
import { activeSession, FakeClock, settle } from './testing'

const MOVE = {
  kind: 'pointer' as const,
  action: 'move' as const,
  button: null,
  point: { x: 160, y: 100, width: 640, height: 400 },
  modifiers: NO_MODIFIERS,
}

async function meteredSession() {
  const clock = new FakeClock()
  const active = await activeSession(undefined, undefined, { clock })
  return { ...active, clock }
}

describe('session stats', () => {
  it('reports the displayed screen and no delay before the peer measures one', async () => {
    const { session, clock } = await meteredSession()
    expect(clock.timers).toBe(1)
    clock.tick()
    expect(session.stats.get()).toMatchObject({
      width: 1280,
      height: 800,
      delayMs: null,
      keyRequests: 0,
      codec: null,
    })
  })

  it("takes the delay from the peer's TestDelay", async () => {
    const s = await meteredSession()
    s.peerSends(
      msg({ case: 'testDelay', value: create(TestDelaySchema, { time: 5n, lastDelay: 83 }) }),
    )
    await settle()
    s.clock.tick()
    expect(s.session.stats.get().delayMs).toBe(83)
  })

  it('counts key frame requests and pointer moves it sends', async () => {
    const s = await meteredSession()
    s.peerSends(msg({ case: 'videoFrame', value: vp9Frame(false) }))
    await settle()
    s.session.input(MOVE)
    s.session.input(MOVE)
    s.session.input({ ...MOVE, action: 'down', button: 'left' })
    s.clock.tick()
    expect(s.session.stats.get()).toMatchObject({ keyRequests: 1, pointerMoves: 2 })
  })

  it('stops once the session is closed', async () => {
    const { session, clock } = await meteredSession()
    clock.tick()
    session.close()
    expect(clock.timers).toBe(0)
    const last = session.stats.get()
    session.input(MOVE)
    clock.tick()
    expect(session.stats.get()).toBe(last)
  })
})
