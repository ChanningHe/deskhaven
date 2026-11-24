import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import {
  EncodedVideoFramesSchema,
  PeerInfoSchema,
  SupportedDecoding_PreferCodec,
  SupportedEncodingSchema,
  SwitchDisplaySchema,
  VideoFrameSchema,
  type Message,
} from '../proto/gen/message_pb.js'
import { FakeDecoders } from '../video/fake-decoder'
import { SAMPLE_FRAMES } from '../video/sample-export'
import { startSession } from './session'
import { FakeClock, FakeHandshake, PEER_INFO, misc, settle } from './testing'

const H26X = { vp9: true, av1: false, vp8: false, h264: true, h265: true }

function h265(...keys: boolean[]): Message {
  return frames('h265s', ...keys)
}

function frames(codec: 'h264s' | 'h265s' | 'vp9s', ...keys: boolean[]): Message {
  return msg({
    case: 'videoFrame',
    value: create(VideoFrameSchema, {
      union: {
        case: codec,
        value: create(EncodedVideoFramesSchema, {
          frames: keys.map((key) => ({ key, data: Uint8Array.of(0, 0, 0, 1, 0x40) })),
        }),
      },
    }),
  })
}

async function sampleSession(active = true) {
  const handshake = new FakeHandshake('relay')
  const created = new FakeDecoders()
  const session = startSession(handshake, {
    createDecoder: created.create,
    clock: new FakeClock(),
    decoders: H26X,
  })
  if (active) {
    const peerInfo = create(PeerInfoSchema, {
      ...PEER_INFO,
      platform: 'Windows',
      version: '1.5.1',
      encoding: create(SupportedEncodingSchema, { h264: true, h265: true }),
    })
    handshake.events.push({ kind: 'state', state: { kind: 'active', peerInfo } })
    await settle()
  }
  const peerSends = async (message: Message) => {
    handshake.peerSends(message)
    await settle()
  }
  const refreshes = () =>
    handshake.sent.filter(
      (m) => m.union.case === 'misc' && m.union.value.union.case === 'refreshVideoDisplay',
    ).length
  return { handshake, created, session, peerSends, refreshes }
}

describe('session video samples', () => {
  it('asks for a key frame and records the frames of the codec it asked for', async () => {
    const s = await sampleSession()
    s.session.setOptions({ decoding: 'h265' })
    const done = s.session.recordSample()
    expect(s.refreshes()).toBe(1)
    expect(s.session.sampling.get()).toEqual({ kind: 'recording', frames: 0 })
    await s.peerSends(h265(true, ...Array<boolean>(SAMPLE_FRAMES - 1).fill(false)))
    const sample = await done
    expect(sample).toMatchObject({
      codec: 'h265s',
      peer: { platform: 'Windows', version: '1.5.1' },
      display: { width: 1280, height: 800 },
    })
    expect(sample?.frames).toHaveLength(SAMPLE_FRAMES)
    expect(s.session.state.get().kind).toBe('active')
  })

  it('records the H264 it asked for up to the fallback, and gives those frames back', async () => {
    const s = await sampleSession()
    s.session.setOptions({ decoding: 'h264' })
    const done = s.session.recordSample()
    await s.peerSends(frames('h264s', true, false, false, false))
    s.created.last.fail()
    await s.peerSends(frames('h264s', true))
    s.created.last.fail()
    const prefers = s.handshake.sent.flatMap((m) =>
      m.union.case === 'misc' && m.union.value.union.case === 'option'
        ? [m.union.value.union.value.supportedDecoding?.prefer]
        : [],
    )
    expect(prefers).toEqual([SupportedDecoding_PreferCodec.H264, SupportedDecoding_PreferCodec.VP9])
    const sample = await done
    expect(sample).toMatchObject({ codec: 'h264s', complete: false })
    expect(sample?.frames.map((f) => f.key)).toEqual([true, false, false, false, true])
    expect(s.session.state.get().kind).toBe('active')
  })

  it('gives nothing back when the peer switches display while recording', async () => {
    const s = await sampleSession()
    const done = s.session.recordSample()
    await s.peerSends(frames('vp9s', true))
    await s.peerSends(
      misc({ case: 'switchDisplay', value: create(SwitchDisplaySchema, { display: 1 }) }),
    )
    expect(await done).toBeNull()
    expect(s.session.sampling.get()).toEqual({ kind: 'idle' })
  })

  it('gives nothing back when the session ends while recording', async () => {
    const s = await sampleSession()
    const done = s.session.recordSample()
    await s.peerSends(h265(true))
    s.session.close()
    expect(await done).toBeNull()
    expect(s.session.sampling.get()).toEqual({ kind: 'idle' })
  })

  it('stops recording on cancel', async () => {
    const s = await sampleSession()
    const done = s.session.recordSample()
    s.session.cancelSample()
    expect(await done).toBeNull()
    expect(s.session.sampling.get()).toEqual({ kind: 'idle' })
  })

  it('records nothing before login', async () => {
    const s = await sampleSession(false)
    expect(await s.session.recordSample()).toBeNull()
    expect(s.refreshes()).toBe(0)
    expect(s.session.sampling.get()).toEqual({ kind: 'idle' })
  })
})
