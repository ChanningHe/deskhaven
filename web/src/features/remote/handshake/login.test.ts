import { describe, expect, it } from 'vitest'

import { create } from '@bufbuild/protobuf'

import {
  ImageQuality,
  OptionMessage_BoolOption,
  OptionMessageSchema,
  SupportedDecoding_PreferCodec,
} from '../proto/gen/message_pb.js'
import {
  DECODER_PROBES,
  loginRequest,
  loginStateForError,
  probeDecoders,
  supportedDecoding,
} from './login'

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

describe('loginRequest', () => {
  const decoding = supportedDecoding({ vp9: true, av1: false, vp8: false })
  if (!decoding) throw new Error('vp9 is decodable')
  const fields = (option = create(OptionMessageSchema)) => ({
    targetId: '1',
    myId: 'a',
    myName: 'A',
    sessionId: 7n,
    decoding,
    option,
  })

  it('adds the settings option to the audio and decoding ones it always sends', () => {
    const option = create(OptionMessageSchema, {
      imageQuality: ImageQuality.Best,
      customImageQuality: 0,
      customFps: 24,
    })
    const message = loginRequest(fields(option), new Uint8Array())
    if (message.union.case !== 'loginRequest') throw new Error('not a LoginRequest')
    expect(message.union.value.option).toMatchObject({
      imageQuality: ImageQuality.Best,
      customFps: 24,
      disableAudio: OptionMessage_BoolOption.Yes,
      supportedDecoding: { abilityVp9: 1, prefer: SupportedDecoding_PreferCodec.VP9 },
    })
    expect(message.union.value.videoAckRequired).toBe(false)
  })

  it('does not let the settings option switch off audio or decoding', () => {
    const option = create(OptionMessageSchema, {
      disableAudio: OptionMessage_BoolOption.No,
      supportedDecoding: undefined,
    })
    const message = loginRequest(fields(option), new Uint8Array())
    if (message.union.case !== 'loginRequest') throw new Error('not a LoginRequest')
    expect(message.union.value.option?.disableAudio).toBe(OptionMessage_BoolOption.Yes)
    expect(message.union.value.option?.supportedDecoding).toMatchObject({ abilityVp9: 1 })
  })

  it('leaves the option it was given untouched', () => {
    const option = create(OptionMessageSchema, { customFps: 24 })
    loginRequest(fields(option), new Uint8Array())
    expect(option.disableAudio).toBe(OptionMessage_BoolOption.NotSet)
    expect(option.supportedDecoding).toBeUndefined()
  })
})
