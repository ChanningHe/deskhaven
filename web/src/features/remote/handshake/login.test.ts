import { describe, expect, it } from 'vitest'

import { SupportedDecoding_PreferCodec } from '../proto/gen/message_pb.js'
import { DECODER_PROBES, loginStateForError, probeDecoders, supportedDecoding } from './login'

describe('supportedDecoding', () => {
  it.each([
    [{ vp9: true, av1: true, vp8: true }, SupportedDecoding_PreferCodec.VP9],
    [{ vp9: false, av1: true, vp8: true }, SupportedDecoding_PreferCodec.AV1],
    [{ vp9: false, av1: false, vp8: true }, SupportedDecoding_PreferCodec.VP8],
  ])('declares each decodable codec and prefers VP9 > AV1 > VP8 (%o)', (support, prefer) => {
    expect(supportedDecoding(support)).toMatchObject({
      abilityVp9: support.vp9 ? 1 : 0,
      abilityAv1: support.av1 ? 1 : 0,
      abilityVp8: support.vp8 ? 1 : 0,
      abilityH264: 0,
      abilityH265: 0,
      prefer,
    })
  })

  it('is null when nothing is decodable', () => {
    expect(supportedDecoding({ vp9: false, av1: false, vp8: false })).toBeNull()
  })
})

describe('probeDecoders', () => {
  it('asks the injected probe about one codec string per codec', async () => {
    const asked: string[] = []
    const support = await probeDecoders((codec) => {
      asked.push(codec)
      return Promise.resolve(codec !== DECODER_PROBES.av1)
    })
    expect(asked.sort()).toEqual(Object.values(DECODER_PROBES).sort())
    expect(support).toEqual({ vp9: true, av1: false, vp8: true })
  })
})

describe('loginStateForError', () => {
  it('ends the session on errors it does not know, including Object.prototype keys', () => {
    expect(loginStateForError('__proto__')).toEqual({ kind: 'failed', reason: '__proto__' })
    expect(loginStateForError('')).toEqual({ kind: 'failed', reason: '' })
  })
})
