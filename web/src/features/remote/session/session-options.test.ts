import { create, type MessageInitShape } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import {
  ImageQuality,
  OptionMessageSchema,
  type Message,
  type OptionMessage,
} from '../proto/gen/message_pb.js'
import { FakeDecoders, vp9Frame } from '../video/fake-decoder'
import { DEFAULT_FPS } from '../video/fps-governor'
import { startSession } from './session'
import { FakeClock, FakeHandshake, activeSession, fakeLinkSession, settle } from './testing'

const option = (init: MessageInitShape<typeof OptionMessageSchema>) =>
  create(OptionMessageSchema, init)

function options(sent: Message[]): OptionMessage[] {
  return sent.flatMap((m) =>
    m.union.case === 'misc' && m.union.value.union.case === 'option'
      ? [m.union.value.union.value]
      : [],
  )
}

const frame = (key: boolean) => msg({ case: 'videoFrame', value: vp9Frame(key) })

async function decoding() {
  const clock = new FakeClock()
  const s = await fakeLinkSession('webrtc', clock)
  for (let i = 0; i < 13; i++) {
    s.handshake.peerSends(frame(i === 0))
    await settle()
    clock.advance(1)
    s.decoders.last.output()
  }
  return { ...s, clock, sent: () => options(s.handshake.sent) }
}

describe('Session.setOptions quality', () => {
  it('sends one Misc.option with the quality fields', async () => {
    const r = await activeSession()
    r.session.setOptions({
      peer: option({ imageQuality: ImageQuality.Best, customImageQuality: 0 }),
    })
    const sent = r.sentAfterLogin()
    expect(sent).toHaveLength(1)
    expect(options(sent)).toMatchObject([
      { imageQuality: ImageQuality.Best, customImageQuality: 0 },
    ])
  })

  it('sends nothing for an empty patch', async () => {
    const r = await activeSession()
    r.session.setOptions({})
    expect(r.sentAfterLogin()).toEqual([])
  })

  it('never lets a patch carry custom_fps to the peer', async () => {
    const r = await activeSession()
    r.session.setOptions({ peer: option({ imageQuality: ImageQuality.Low, customFps: 99 }) })
    expect(options(r.sentAfterLogin())).toMatchObject([
      { imageQuality: ImageQuality.Low, customFps: 0 },
    ])
  })
})

describe('Session.setOptions maxFps', () => {
  it('moves the governor ceiling and sends nothing itself', async () => {
    const s = await decoding()
    s.clock.tick()
    expect(s.sent().map((o) => o.customFps)).toEqual([DEFAULT_FPS])

    s.session.setOptions({ maxFps: 12 })
    expect(s.sent()).toHaveLength(1)

    s.clock.tick()
    expect(s.sent().map((o) => o.customFps)).toEqual([DEFAULT_FPS, 12])
  })

  it('keeps custom_fps out of the quality message when both change', async () => {
    const s = await decoding()
    s.clock.tick()
    s.session.setOptions({ peer: option({ imageQuality: ImageQuality.Low }), maxFps: 20 })
    const [, quality] = s.sent()
    expect(quality).toMatchObject({ imageQuality: ImageQuality.Low, customFps: 0 })
    s.clock.tick()
    expect(s.sent().map((o) => o.customFps)).toEqual([DEFAULT_FPS, 0, 20])
  })
})

describe('Session.setOptions outside an active session', () => {
  const patch = { peer: option({ imageQuality: ImageQuality.Best }), maxFps: 20 }

  it('is ignored while connecting', () => {
    const handshake = new FakeHandshake('relay')
    const session = startSession(handshake, { createDecoder: new FakeDecoders().create })
    expect(session.state.get().kind).toBe('connecting')
    expect(() => {
      session.setOptions(patch)
    }).not.toThrow()
    expect(handshake.sent).toEqual([])
  })

  it('is ignored once closed', async () => {
    const s = await fakeLinkSession('relay', new FakeClock())
    s.session.close()
    const before = s.handshake.sent.length
    expect(s.session.state.get().kind).toBe('closed')
    expect(() => {
      s.session.setOptions(patch)
    }).not.toThrow()
    expect(s.handshake.sent).toHaveLength(before)
  })
})
