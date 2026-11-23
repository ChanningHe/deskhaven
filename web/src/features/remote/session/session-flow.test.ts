import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import type { LinkKind } from '../handshake/handshake'
import { SwitchDisplaySchema, VideoFrameSchema, type Message } from '../proto/gen/message_pb.js'
import { vp9Frame } from '../video/fake-decoder'
import { DEFAULT_FPS } from '../video/fps-governor'
import { activeSession, FakeClock, fakeLinkSession, misc, settle } from './testing'

function miscCases(sent: Message[]): (string | undefined)[] {
  return sent.flatMap((m) => (m.union.case === 'misc' ? [m.union.value.union.case] : []))
}

function fpsSent(sent: Message[]): number[] {
  return sent.flatMap((m) => {
    if (m.union.case !== 'misc' || m.union.value.union.case !== 'option') return []
    const { customFps, ...rest } = m.union.value.union.value
    expect(rest).toMatchObject({ imageQuality: 0, customImageQuality: 0, disableAudio: 0 })
    return [customFps]
  })
}

const frame = (key: boolean) => msg({ case: 'videoFrame', value: vp9Frame(key) })

async function sessionOver(link: LinkKind) {
  const clock = new FakeClock()
  if (link === 'relay') {
    const r = await activeSession(undefined, undefined, { clock })
    return { ...r, clock, sent: () => r.sentAfterLogin() }
  }
  const s = await fakeLinkSession(link, clock)
  return {
    ...s,
    clock,
    peerSends: (message: Message) => {
      s.handshake.peerSends(message)
    },
    sent: () => s.handshake.sent,
  }
}

async function decodeEach(s: Awaited<ReturnType<typeof sessionOver>>, serviceMs: number) {
  for (let i = 0; i < 13; i++) {
    s.peerSends(frame(i === 0))
    await settle()
    s.clock.advance(serviceMs)
    s.decoders.last.output()
  }
}

describe.each(['relay', 'webrtc'] as const)('video flow over %s', (link) => {
  it('never sends video_received, whatever becomes of the frames', async () => {
    const s = await sessionOver(link)
    s.peerSends(frame(false))
    s.peerSends(msg({ case: 'videoFrame', value: create(VideoFrameSchema) }))
    s.peerSends(frame(true))
    s.peerSends(frame(false))
    await settle()
    s.decoders.last.output(0)
    s.decoders.last.fail()
    s.peerSends(frame(true))
    s.peerSends(misc({ case: 'switchDisplay', value: create(SwitchDisplaySchema, { display: 0 }) }))
    await settle()
    s.session.close()

    const sent = miscCases(s.sent())
    expect(sent).not.toContain('videoReceived')
    expect(sent).toContain('refreshVideoDisplay')
    expect(sent).toContain('closeReason')
  })

  it(`asks for no more than ${DEFAULT_FPS} fps`, async () => {
    const s = await sessionOver(link)
    await decodeEach(s, 1)
    s.clock.tick()
    expect(fpsSent(s.sent())).toEqual([DEFAULT_FPS])
  })
})

describe('video flow fps margin', () => {
  it.each([
    ['relay', 16, 8],
    ['webrtc', 18, 9],
  ] as const)(
    'over %s asks for %i fps at 20 decoded, and halves it while frames pile up',
    async (link, fps, halved) => {
      const s = await sessionOver(link)
      await decodeEach(s, 50)
      s.clock.tick()
      expect(fpsSent(s.sent())).toEqual([fps])

      for (let i = 0; i < 20; i++) s.peerSends(frame(false))
      await settle()
      s.clock.tick()
      s.clock.tick()
      expect(fpsSent(s.sent())).toEqual([fps, halved])
    },
  )
})
