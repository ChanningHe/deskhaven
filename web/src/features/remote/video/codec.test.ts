import { create } from '@bufbuild/protobuf'
import { describe, expect, it } from 'vitest'

import {
  SupportedDecoding_PreferCodec,
  SupportedEncodingSchema,
  type SupportedDecoding,
} from '../proto/gen/message_pb.js'
import type { DecoderSupport } from '../handshake/login'
import { CodecSwitch, SWITCH_ERRORS, SWITCH_TIMEOUT_MS, type CodecView } from './codec'

const ALL: DecoderSupport = { vp9: true, av1: true, vp8: true, h264: true, h265: true }
const { VP9, AV1, VP8, H264, H265 } = SupportedDecoding_PreferCodec
const NO_H26X = { h264: 'peer', h265: 'peer' } as const

function setup(
  support: DecoderSupport = ALL,
  encoding: { av1: boolean; vp8: boolean; h264?: boolean; h265?: boolean } = {
    av1: true,
    vp8: true,
  },
  onChange: (codecs: CodecSwitch) => void = () => undefined,
) {
  let now = 0
  const sent: SupportedDecoding[] = []
  const views: CodecView[] = []
  const calls = { refreshes: 0 }
  const codecs: CodecSwitch = new CodecSwitch({
    support,
    now: () => now,
    send: (decoding) => sent.push(decoding),
    refresh: () => calls.refreshes++,
    onChange: (view) => {
      views.push(view)
      onChange(codecs)
    },
  })
  codecs.peerEncodes(create(SupportedEncodingSchema, encoding))
  return {
    codecs,
    sent,
    views,
    calls,
    advance: (ms: number) => {
      now += ms
      codecs.tick()
    },
  }
}

describe('CodecSwitch support', () => {
  it('starts out on VP9 with only VP9 to pick until the peer says what it encodes', () => {
    const codecs = new CodecSwitch({
      support: ALL,
      now: () => 0,
      send: () => undefined,
      refresh: () => undefined,
      onChange: () => undefined,
    })
    expect(codecs.view).toEqual({
      support: { vp9: 'ok', av1: 'peer', vp8: 'peer', ...NO_H26X },
      requested: 'vp9',
      receiving: null,
    })
  })

  it('offers what both this browser decodes and the peer encodes', () => {
    const { codecs } = setup({ ...ALL, av1: false }, { av1: true, vp8: false })
    expect(codecs.view.support).toEqual({ vp9: 'ok', av1: 'browser', vp8: 'peer', ...NO_H26X })
  })

  it('takes VP9 as encodable whatever the peer says, and a missing encoding as VP9 only', () => {
    const { codecs } = setup(ALL, { av1: false, vp8: false })
    expect(codecs.view.support).toEqual({ vp9: 'ok', av1: 'peer', vp8: 'peer', ...NO_H26X })
    codecs.peerEncodes(undefined)
    expect(codecs.view.support).toEqual({ vp9: 'ok', av1: 'peer', vp8: 'peer', ...NO_H26X })
  })

  it('follows each update of what the peer encodes', () => {
    const { codecs, views } = setup(ALL, { av1: false, vp8: false })
    codecs.peerUsable(create(SupportedEncodingSchema, { av1: true, vp8: false, h264: true }))
    expect(codecs.view.support).toEqual({ vp9: 'ok', av1: 'ok', vp8: 'peer', ...NO_H26X })
    const seen = views.length
    codecs.peerUsable(create(SupportedEncodingSchema, { av1: true, vp8: false }))
    expect(views).toHaveLength(seen)
  })

  it("offers H264 and H265 on the peer's PeerInfo, which says what it can encode", () => {
    const { codecs } = setup(ALL, { av1: true, vp8: true, h264: true, h265: false })
    expect(codecs.view.support).toMatchObject({ h264: 'ok', h265: 'peer' })
  })

  it('takes H264 and H265 in Misc.supported_encoding as only what we declared', () => {
    const { codecs } = setup(ALL, { av1: true, vp8: true, h264: true, h265: true })
    codecs.peerUsable(create(SupportedEncodingSchema, { av1: true, vp8: true }))
    expect(codecs.view.support).toMatchObject({ h264: 'ok', h265: 'ok' })
    codecs.peerEncodes(create(SupportedEncodingSchema, { av1: true, vp8: true, h264: false }))
    expect(codecs.view.support).toMatchObject({ h264: 'peer', h265: 'peer' })
  })

  it('names the device first when neither side can', () => {
    const { codecs } = setup({ ...ALL, h265: false }, { av1: true, vp8: true, h264: false })
    expect(codecs.view.support).toMatchObject({ h264: 'peer', h265: 'peer' })
  })
})

describe('CodecSwitch.want', () => {
  it('asks for the codec with the abilities it logged in with', () => {
    const { codecs, sent } = setup()
    codecs.want('av1')
    expect(sent).toHaveLength(1)
    expect(sent[0]).toMatchObject({ abilityVp9: 1, abilityAv1: 1, abilityVp8: 1, prefer: AV1 })
    expect(codecs.view.requested).toBe('av1')
  })

  it('asks for nothing the peer cannot encode, and asks once it can', () => {
    const { codecs, sent } = setup(ALL, { av1: false, vp8: true })
    codecs.want('av1')
    expect(sent).toEqual([])
    expect(codecs.view.requested).toBe('vp9')
    codecs.peerEncodes(create(SupportedEncodingSchema, { av1: true, vp8: true }))
    expect(sent.map((d) => d.prefer)).toEqual([AV1])
  })

  it('asks for nothing this browser cannot decode', () => {
    const { codecs, sent } = setup({ ...ALL, av1: false })
    codecs.want('av1')
    expect(sent).toEqual([])
  })

  it('asks again only for a change', () => {
    const { codecs, sent } = setup()
    codecs.want('vp9')
    codecs.want('vp8')
    codecs.want('vp8')
    codecs.want('vp9')
    expect(sent.map((d) => d.prefer)).toEqual([VP8, VP9])
  })

  it.each([
    ['h264', H264, { abilityH264: 1, abilityH265: 0 }],
    ['h265', H265, { abilityH264: 0, abilityH265: 1 }],
  ] as const)('declares %s only while asking for it', (codec, prefer, abilities) => {
    const { codecs, sent } = setup(ALL, { av1: true, vp8: true, h264: true, h265: true })
    codecs.want(codec)
    codecs.want('av1')
    codecs.want('vp9')
    expect(sent).toHaveLength(3)
    expect(sent[0]).toMatchObject({
      abilityVp9: 1,
      abilityAv1: 1,
      abilityVp8: 1,
      ...abilities,
      prefer,
    })
    for (const decoding of sent.slice(1)) {
      expect(decoding).toMatchObject({ abilityH264: 0, abilityH265: 0 })
    }
    expect(sent.map((d) => d.prefer)).toEqual([prefer, AV1, VP9])
  })

  it('withdraws H264 when it falls back from it', () => {
    const { codecs, sent } = setup(ALL, { av1: true, vp8: true, h264: true })
    codecs.want('h264')
    codecs.received('h264')
    for (let i = 0; i < SWITCH_ERRORS; i++) codecs.failed('h264')
    expect(sent.map((d) => d.prefer)).toEqual([H264, VP9])
    expect(sent[1]).toMatchObject({ abilityVp9: 1, abilityH264: 0, abilityH265: 0 })
    expect(codecs.view).toMatchObject({ requested: 'vp9', failed: 'h264' })
    expect(codecs.view.support.h264).toBe('failed')
    expect(codecs.accepts('h264')).toBe(false)
  })
})

describe('CodecSwitch fallback', () => {
  it('keeps a codec whose frames come out of the decoder', () => {
    const { codecs, sent, advance } = setup()
    codecs.want('av1')
    codecs.received('av1')
    codecs.decoded('av1')
    advance(SWITCH_TIMEOUT_MS * 3)
    expect(sent.map((d) => d.prefer)).toEqual([AV1])
    expect(codecs.view).toMatchObject({ requested: 'av1', receiving: 'av1' })
    expect(codecs.view.failedAt).toBeUndefined()
  })

  it(`goes back to VP9 when no frame of the codec comes out within ${SWITCH_TIMEOUT_MS} ms`, () => {
    const { codecs, sent, advance } = setup()
    codecs.received('vp9')
    codecs.want('av1')
    codecs.received('av1')
    advance(SWITCH_TIMEOUT_MS - 1)
    expect(sent).toHaveLength(1)
    advance(1)
    expect(sent).toHaveLength(2)
    expect(sent[1]).toMatchObject({ abilityVp9: 1, abilityAv1: 0, abilityVp8: 1, prefer: VP9 })
    expect(codecs.view).toEqual({
      support: { vp9: 'ok', av1: 'failed', vp8: 'ok', ...NO_H26X },
      requested: 'vp9',
      receiving: 'av1',
      failed: 'av1',
      failedAt: SWITCH_TIMEOUT_MS,
    })
  })

  it('goes back to VP9 when the peer never switches', () => {
    const { codecs, sent, advance } = setup()
    codecs.received('vp9')
    codecs.want('vp8')
    for (let t = 0; t < SWITCH_TIMEOUT_MS; t += 1000) {
      codecs.received('vp9')
      codecs.decoded('vp9')
      advance(1000)
    }
    expect(sent.map((d) => d.prefer)).toEqual([VP8, VP9])
    expect(codecs.view.failed).toBe('vp8')
    expect(codecs.view.support.vp8).toBe('ignored')
  })

  it('takes old frames that land only early in the wait as in flight, not as a refusal', () => {
    const { codecs, sent, calls, advance } = setup()
    codecs.received('vp9')
    codecs.want('av1')
    codecs.received('vp9')
    codecs.decoded('vp9')
    for (let i = 0; i < 3; i++) advance(SWITCH_TIMEOUT_MS)
    expect(sent.map((d) => d.prefer)).toEqual([AV1])
    expect(calls.refreshes).toBe(3)
    expect(codecs.view.failedAt).toBeUndefined()
  })

  it('takes an old frame in the second half of the wait as the peer going on with it', () => {
    const { codecs, sent, calls, advance } = setup()
    codecs.want('av1')
    advance(SWITCH_TIMEOUT_MS / 2)
    codecs.received('vp9')
    advance(SWITCH_TIMEOUT_MS / 2)
    expect(calls.refreshes).toBe(0)
    expect(sent.map((d) => d.prefer)).toEqual([AV1, VP9])
    expect(codecs.view.support.av1).toBe('ignored')
  })

  it('takes a silent peer as a still screen: asks for a refresh and waits again', () => {
    const { codecs, sent, calls, advance } = setup()
    codecs.want('av1')
    for (let i = 0; i < 3; i++) advance(SWITCH_TIMEOUT_MS)
    expect(sent.map((d) => d.prefer)).toEqual([AV1])
    expect(calls.refreshes).toBe(3)
    expect(codecs.view.failedAt).toBeUndefined()
    expect(codecs.view.requested).toBe('av1')
  })

  it('judges the codec once frames come after a silence', () => {
    const { codecs, sent, calls, advance } = setup()
    codecs.want('av1')
    advance(SWITCH_TIMEOUT_MS)
    codecs.received('av1')
    advance(SWITCH_TIMEOUT_MS - 1)
    expect(sent).toHaveLength(1)
    advance(1)
    expect(calls.refreshes).toBe(1)
    expect(sent.map((d) => d.prefer)).toEqual([AV1, VP9])
    expect(codecs.view.support.av1).toBe('failed')
  })

  it('asks for VP9 once when its own listener reacts to the fallback', () => {
    const { codecs, sent, advance } = setup(ALL, { av1: true, vp8: true }, (c) => {
      if (c.view.failed !== undefined) c.want('vp9')
    })
    codecs.want('av1')
    codecs.received('av1')
    advance(SWITCH_TIMEOUT_MS)
    expect(sent.map((d) => d.prefer)).toEqual([AV1, VP9])
  })

  it('falls back to the codec it logged in with where VP9 is not decodable', () => {
    const { codecs, sent, advance } = setup({ ...ALL, vp9: false })
    expect(codecs.view.requested).toBe('av1')
    for (let i = 0; i < SWITCH_ERRORS; i++) codecs.failed('av1')
    expect(sent).toEqual([])
    codecs.want('vp8')
    codecs.received('vp8')
    advance(SWITCH_TIMEOUT_MS)
    expect(sent.map((d) => d.prefer)).toEqual([VP8, AV1])
    expect(sent[1]).toMatchObject({ abilityVp9: 0, abilityAv1: 1, abilityVp8: 0 })
    expect(codecs.view).toMatchObject({ requested: 'av1', failed: 'vp8' })
  })

  it(`goes back to VP9 after ${SWITCH_ERRORS} decode errors in a row`, () => {
    const { codecs, sent } = setup()
    codecs.want('av1')
    codecs.received('av1')
    for (let i = 0; i < SWITCH_ERRORS - 1; i++) codecs.failed('av1')
    expect(sent).toHaveLength(1)
    codecs.failed('av1')
    expect(sent.map((d) => d.prefer)).toEqual([AV1, VP9])
    expect(codecs.view.support.av1).toBe('failed')
  })

  it('counts decode errors only in a row, and also once the codec has worked', () => {
    const { codecs, sent, advance } = setup()
    const failures = () => {
      for (let i = 0; i < SWITCH_ERRORS - 1; i++) codecs.failed('av1')
    }
    codecs.want('av1')
    failures()
    codecs.decoded('av1')
    failures()
    advance(SWITCH_TIMEOUT_MS)
    expect(sent).toHaveLength(1)
    codecs.failed('av1')
    expect(sent.map((d) => d.prefer)).toEqual([AV1, VP9])
  })

  it('never falls back from VP9', () => {
    const { codecs, sent, advance } = setup()
    for (let i = 0; i < SWITCH_ERRORS * 2; i++) codecs.failed('vp9')
    advance(SWITCH_TIMEOUT_MS)
    expect(sent).toEqual([])
    expect(codecs.view.failedAt).toBeUndefined()
  })

  it('takes a third codec in the second half of the wait as the peer refusing H264', () => {
    const { codecs, sent, advance } = setup(ALL, { av1: true, vp8: true, h264: true })
    codecs.received('vp9')
    codecs.want('h264')
    advance(SWITCH_TIMEOUT_MS / 2)
    codecs.received('av1')
    codecs.decoded('av1')
    advance(SWITCH_TIMEOUT_MS / 2)
    expect(sent.map((d) => d.prefer)).toEqual([H264, VP9])
    expect(sent[1]).toMatchObject({ abilityH264: 0 })
    expect(codecs.view.support.h264).toBe('ignored')
    expect(codecs.view.failed).toBe('h264')
  })

  it('shows another codec the peer picked instead, without falling back', () => {
    const { codecs, sent, advance } = setup()
    codecs.received('vp9')
    codecs.want('av1')
    codecs.received('vp8')
    codecs.decoded('vp8')
    advance(SWITCH_TIMEOUT_MS * 2)
    expect(sent.map((d) => d.prefer)).toEqual([AV1])
    expect(codecs.view).toMatchObject({ requested: 'av1', receiving: 'vp8' })
    expect(codecs.view.failedAt).toBeUndefined()
  })

  it('stops watching a codec the peer stops offering', () => {
    const { codecs, sent, advance } = setup()
    codecs.want('av1')
    codecs.peerEncodes(create(SupportedEncodingSchema, { av1: false, vp8: true }))
    advance(SWITCH_TIMEOUT_MS)
    expect(sent.map((d) => d.prefer)).toEqual([AV1])
    expect(codecs.view.support.av1).toBe('peer')
  })

  it('stops watching when the user goes back to VP9', () => {
    const { codecs, sent, advance } = setup()
    codecs.want('av1')
    codecs.want('vp9')
    advance(SWITCH_TIMEOUT_MS)
    expect(sent.map((d) => d.prefer)).toEqual([AV1, VP9])
    expect(codecs.view.failedAt).toBeUndefined()
  })

  it('drops frames of a failed codec and never asks for it again in this session', () => {
    const { codecs, sent, advance } = setup()
    expect(codecs.accepts('av1')).toBe(true)
    codecs.want('av1')
    codecs.received('av1')
    advance(SWITCH_TIMEOUT_MS)
    expect(codecs.accepts('av1')).toBe(false)
    expect(codecs.accepts('vp9')).toBe(true)
    codecs.want('av1')
    codecs.peerEncodes(create(SupportedEncodingSchema, { av1: true, vp8: true }))
    expect(sent.map((d) => d.prefer)).toEqual([AV1, VP9])
    codecs.want('vp8')
    expect(sent.at(-1)).toMatchObject({ abilityAv1: 0, prefer: VP8 })
  })

  it('drops frames this browser cannot decode', () => {
    const { codecs } = setup({ ...ALL, av1: false })
    expect(codecs.accepts('av1')).toBe(false)
  })
})
