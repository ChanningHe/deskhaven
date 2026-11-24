import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import { msg } from '../__fixtures__/harness'
import type { DecoderSupport } from '../handshake/login'
import {
  EncodedVideoFramesSchema,
  PeerInfoSchema,
  SupportedDecoding_PreferCodec,
  SupportedEncodingSchema,
  VideoFrameSchema,
  type Message,
  type SupportedDecoding,
} from '../proto/gen/message_pb.js'
import { SWITCH_TIMEOUT_MS } from '../video/codec'
import { FakeDecoders, vp9Frame } from '../video/fake-decoder'
import { startSession } from './session'
import { FakeClock, FakeHandshake, PEER_INFO, misc, settle } from './testing'

const ALL: DecoderSupport = { vp9: true, av1: true, vp8: true, h264: true, h265: true }
const { VP9, AV1, H264 } = SupportedDecoding_PreferCodec
const NO_H26X = { h264: 'peer', h265: 'peer' } as const

function decodings(sent: Message[]): SupportedDecoding[] {
  return sent.flatMap((m) =>
    m.union.case === 'misc' &&
    m.union.value.union.case === 'option' &&
    m.union.value.union.value.supportedDecoding
      ? [m.union.value.union.value.supportedDecoding]
      : [],
  )
}

function videoFrame(codec: 'av1s' | 'vp8s' | 'h264s', key: boolean): Message {
  return msg({
    case: 'videoFrame',
    value: create(VideoFrameSchema, {
      union: {
        case: codec,
        value: create(EncodedVideoFramesSchema, { frames: [{ key, data: Uint8Array.of(1) }] }),
      },
    }),
  })
}

async function codecSession(
  encoding: { av1: boolean; vp8: boolean; h264?: boolean } | null = { av1: true, vp8: true },
  decoders: DecoderSupport = ALL,
) {
  const clock = new FakeClock()
  const handshake = new FakeHandshake('relay')
  const created = new FakeDecoders()
  const session = startSession(handshake, { createDecoder: created.create, clock, decoders })
  const peerInfo = create(PeerInfoSchema, {
    ...PEER_INFO,
    version: '1.5.0',
    encoding: encoding ? create(SupportedEncodingSchema, encoding) : undefined,
  })
  handshake.events.push({ kind: 'state', state: { kind: 'active', peerInfo } })
  await settle()
  const peerSends = async (message: Message) => {
    handshake.peerSends(message)
    await settle()
  }
  return { clock, handshake, created, session, peerSends, sent: () => decodings(handshake.sent) }
}

describe('session codecs', () => {
  it("offers what the browser decodes and the peer's PeerInfo says it encodes", async () => {
    const s = await codecSession({ av1: false, vp8: true }, { ...ALL, vp8: false })
    expect(s.session.codecs.get()).toEqual({
      support: { vp9: 'ok', av1: 'peer', vp8: 'browser', ...NO_H26X },
      requested: 'vp9',
      receiving: null,
    })
  })

  it('takes a peer without an encoding in its PeerInfo as VP9 only', async () => {
    const s = await codecSession(null)
    expect(s.session.codecs.get().support).toEqual({
      vp9: 'ok',
      av1: 'peer',
      vp8: 'peer',
      ...NO_H26X,
    })
  })

  it('follows Misc.supported_encoding', async () => {
    const s = await codecSession({ av1: false, vp8: false })
    await s.peerSends(
      misc({
        case: 'supportedEncoding',
        value: create(SupportedEncodingSchema, { av1: true, vp8: false, h265: true }),
      }),
    )
    expect(s.session.codecs.get().support).toEqual({
      vp9: 'ok',
      av1: 'ok',
      vp8: 'peer',
      ...NO_H26X,
    })
  })

  it('keeps H264 offered by PeerInfo when Misc.supported_encoding, our usable set, leaves it out', async () => {
    const s = await codecSession({ av1: true, vp8: true, h264: true })
    expect(s.session.codecs.get().support.h264).toBe('ok')
    await s.peerSends(
      misc({
        case: 'supportedEncoding',
        value: create(SupportedEncodingSchema, { av1: true, vp8: true, h264: false }),
      }),
    )
    expect(s.session.codecs.get().support.h264).toBe('ok')
  })

  it('asks for H264 declaring it, and decodes its Annex B frames', async () => {
    const s = await codecSession({ av1: true, vp8: true, h264: true })
    s.session.setOptions({ decoding: 'h264' })
    expect(s.sent()).toHaveLength(1)
    expect(s.sent()[0]).toMatchObject({
      abilityVp9: 1,
      abilityH264: 1,
      abilityH265: 0,
      prefer: H264,
    })
    await s.peerSends(videoFrame('h264s', true))
    s.created.last.output()
    s.clock.tick()
    expect(s.created.last.codec).toBe('avc1.640033')
    expect(s.session.stats.get().codec).toBe('avc1.640033')
    expect(s.session.codecs.get()).toMatchObject({ requested: 'h264', receiving: 'h264' })
  })

  it('switches from VP9 to AV1 with one Misc.option, abilities as at login', async () => {
    const s = await codecSession()
    s.session.setOptions({ decoding: 'av1' })
    expect(s.sent()).toHaveLength(1)
    expect(s.sent()[0]).toMatchObject({
      abilityVp9: 1,
      abilityAv1: 1,
      abilityVp8: 1,
      abilityH264: 0,
      abilityH265: 0,
      prefer: AV1,
    })
    expect(s.handshake.sent).toHaveLength(1)
    const option = s.handshake.sent[0]
    if (option?.union.case !== 'misc' || option.union.value.union.case !== 'option') {
      throw new Error('not a Misc.option')
    }
    expect(option.union.value.union.value).toMatchObject({ customFps: 0, customImageQuality: 0 })
  })

  it('sends nothing for a codec the peer does not encode', async () => {
    const s = await codecSession({ av1: false, vp8: true })
    s.session.setOptions({ decoding: 'av1' })
    expect(s.handshake.sent).toEqual([])
    expect(s.session.codecs.get().requested).toBe('vp9')
  })

  it('shows the codec of the frames it receives in the stats', async () => {
    const s = await codecSession()
    s.session.setOptions({ decoding: 'vp8' })
    await s.peerSends(msg({ case: 'videoFrame', value: vp9Frame(true) }))
    s.clock.tick()
    expect(s.session.stats.get().codec).toBe('vp09.00.10.08')
    await s.peerSends(videoFrame('vp8s', true))
    s.created.last.output()
    s.clock.tick()
    expect(s.session.stats.get().codec).toBe('vp8')
    expect(s.session.codecs.get()).toMatchObject({ requested: 'vp8', receiving: 'vp8' })
  })

  it('goes back to VP9 when the new codec puts out no frame in time', async () => {
    const s = await codecSession()
    s.session.setOptions({ decoding: 'av1' })
    await s.peerSends(videoFrame('av1s', true))
    for (let t = 0; t < SWITCH_TIMEOUT_MS; t += 1000) s.clock.tick()
    expect(s.sent().map((d) => d.prefer)).toEqual([AV1, VP9])
    expect(s.sent()[1]).toMatchObject({ abilityAv1: 0, abilityVp9: 1, abilityVp8: 1 })
    expect(s.session.codecs.get()).toMatchObject({ requested: 'vp9', failed: 'av1' })
    expect(s.session.state.get().kind).toBe('active')
  })

  it('asks a silent peer for a refresh instead of giving the codec up', async () => {
    const s = await codecSession()
    s.session.setOptions({ decoding: 'av1' })
    for (let t = 0; t < SWITCH_TIMEOUT_MS * 2; t += 1000) s.clock.tick()
    expect(s.sent().map((d) => d.prefer)).toEqual([AV1])
    const refreshes = s.handshake.sent.filter(
      (m) => m.union.case === 'misc' && m.union.value.union.case === 'refreshVideoDisplay',
    )
    expect(refreshes).toHaveLength(2)
    expect(s.session.codecs.get().failedAt).toBeUndefined()
  })

  it('goes back to VP9 on decode errors before the decoder gives up the session', async () => {
    const s = await codecSession()
    s.session.setOptions({ decoding: 'av1' })
    await s.peerSends(videoFrame('av1s', true))
    s.created.last.fail()
    await s.peerSends(videoFrame('av1s', true))
    s.created.last.fail()
    expect(s.sent().map((d) => d.prefer)).toEqual([AV1, VP9])
    await s.peerSends(videoFrame('av1s', true))
    await s.peerSends(msg({ case: 'videoFrame', value: vp9Frame(true) }))
    s.created.last.output()
    expect(s.created.created.map((d) => d.codec)).toEqual([
      'av01.0.04M.08',
      'av01.0.04M.08',
      'vp09.00.10.08',
    ])
    expect(s.session.state.get().kind).toBe('active')
  })
})
