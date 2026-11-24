import { describe, expect, it } from 'vitest'

import { create } from '@bufbuild/protobuf'

import {
  ImageQuality,
  OptionMessage_BoolOption,
  OptionMessageSchema,
  SupportedDecoding_PreferCodec,
} from '../proto/gen/message_pb.js'
import { loginOption } from '../settings/protocol'
import { DEFAULT_SETTINGS } from '../settings/settings'
import {
  DECODER_PROBES,
  LOGIN_CODECS,
  loginRequest,
  loginStateForError,
  VIDEO_CODECS,
  preferring,
  probeDecoders,
  supportedDecoding,
} from './login'

const NO_H26X = { h264: false, h265: false }

describe('supportedDecoding', () => {
  const ALL = { vp9: true, av1: true, vp8: true, h264: true, h265: true }

  it.each([
    [ALL, SupportedDecoding_PreferCodec.VP9],
    [{ ...ALL, vp9: false }, SupportedDecoding_PreferCodec.AV1],
    [{ ...ALL, vp9: false, av1: false }, SupportedDecoding_PreferCodec.VP8],
  ])('declares each decodable login codec and prefers VP9 > AV1 > VP8 (%o)', (support, prefer) => {
    expect(supportedDecoding(support, LOGIN_CODECS)).toMatchObject({
      abilityVp9: support.vp9 ? 1 : 0,
      abilityAv1: support.av1 ? 1 : 0,
      abilityVp8: support.vp8 ? 1 : 0,
      abilityH264: 0,
      abilityH265: 0,
      prefer,
    })
  })

  it('declares only the codecs in `declared`', () => {
    expect(supportedDecoding(ALL, ['vp9', 'vp8'])).toMatchObject({
      abilityVp9: 1,
      abilityAv1: 0,
      abilityVp8: 1,
      prefer: SupportedDecoding_PreferCodec.VP9,
    })
  })

  it('logs in declaring VP9, AV1 and VP8 only: H264 and H265 wait until picked', () => {
    expect(LOGIN_CODECS).toEqual(['vp9', 'av1', 'vp8'])
    expect(VIDEO_CODECS).toEqual(['vp9', 'av1', 'vp8', 'h264', 'h265'])
  })

  it.each([
    ['h264', { abilityH264: 1, abilityH265: 0 }],
    ['h265', { abilityH264: 0, abilityH265: 1 }],
  ] as const)('declares %s when asked to, still preferring VP9', (codec, abilities) => {
    expect(supportedDecoding(ALL, [...LOGIN_CODECS, codec])).toMatchObject({
      abilityVp9: 1,
      ...abilities,
      prefer: SupportedDecoding_PreferCodec.VP9,
    })
  })

  it('does not declare H264 or H265 this browser cannot decode', () => {
    expect(supportedDecoding({ ...ALL, h264: false }, VIDEO_CODECS)).toMatchObject({
      abilityH264: 0,
      abilityH265: 1,
    })
  })

  it('is null when nothing is decodable', () => {
    const none = { vp9: false, av1: false, vp8: false, h264: false, h265: false }
    expect(supportedDecoding(none, VIDEO_CODECS)).toBeNull()
    expect(supportedDecoding(ALL, [])).toBeNull()
  })
})

describe('preferring', () => {
  it.each([
    ['vp9', SupportedDecoding_PreferCodec.VP9],
    ['av1', SupportedDecoding_PreferCodec.AV1],
    ['vp8', SupportedDecoding_PreferCodec.VP8],
    ['h264', SupportedDecoding_PreferCodec.H264],
    ['h265', SupportedDecoding_PreferCodec.H265],
  ] as const)('asks for %s and keeps the abilities', (codec, prefer) => {
    const decoding = supportedDecoding(
      { vp9: true, av1: true, vp8: false, ...NO_H26X },
      VIDEO_CODECS,
    )
    if (!decoding) throw new Error('decodable')
    expect(preferring(decoding, codec)).toMatchObject({
      abilityVp9: 1,
      abilityAv1: 1,
      abilityVp8: 0,
      prefer,
    })
    expect(decoding.prefer).toBe(SupportedDecoding_PreferCodec.VP9)
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
    expect(support).toEqual({ vp9: true, av1: false, vp8: true, h264: true, h265: true })
  })

  it('probes H264 High and H265 Main, with no description: the Annex B form', () => {
    expect(DECODER_PROBES.h264).toBe('avc1.640033')
    expect(DECODER_PROBES.h265).toBe('hvc1.1.6.L153.B0')
  })
})

describe('loginStateForError', () => {
  it('ends the session on errors it does not know, including Object.prototype keys', () => {
    expect(loginStateForError('__proto__')).toEqual({ kind: 'failed', reason: '__proto__' })
    expect(loginStateForError('')).toEqual({ kind: 'failed', reason: '' })
  })
})

describe('loginRequest', () => {
  const decoding = supportedDecoding(
    { vp9: true, av1: false, vp8: false, ...NO_H26X },
    VIDEO_CODECS,
  )
  if (!decoding) throw new Error('vp9 is decodable')
  const fields = (option = create(OptionMessageSchema)) => ({
    targetId: '1',
    myId: 'a',
    myName: 'A',
    sessionId: 7n,
    decoding,
    option,
  })

  it('adds the decoding option it always sends to the settings option, audio included', () => {
    const option = create(OptionMessageSchema, {
      imageQuality: ImageQuality.Best,
      customImageQuality: 0,
      customFps: 24,
      disableAudio: OptionMessage_BoolOption.Yes,
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

  it('leaves audio to the settings option and does not let it switch off decoding', () => {
    const option = create(OptionMessageSchema, {
      disableAudio: OptionMessage_BoolOption.No,
      supportedDecoding: undefined,
    })
    const message = loginRequest(fields(option), new Uint8Array())
    if (message.union.case !== 'loginRequest') throw new Error('not a LoginRequest')
    expect(message.union.value.option?.disableAudio).toBe(OptionMessage_BoolOption.No)
    expect(message.union.value.option?.supportedDecoding).toMatchObject({ abilityVp9: 1 })
  })

  it('prefers VP9 at login whatever codec the settings ask for', () => {
    const decoding = supportedDecoding(
      { vp9: true, av1: true, vp8: true, ...NO_H26X },
      VIDEO_CODECS,
    )
    if (!decoding) throw new Error('decodable')
    const option = loginOption({ ...DEFAULT_SETTINGS, codec: 'av1' })
    const message = loginRequest({ ...fields(option), decoding }, new Uint8Array())
    if (message.union.case !== 'loginRequest') throw new Error('not a LoginRequest')
    expect(message.union.value.option?.supportedDecoding).toMatchObject({
      abilityVp9: 1,
      abilityAv1: 1,
      abilityVp8: 1,
      prefer: SupportedDecoding_PreferCodec.VP9,
    })
  })

  it('leaves the option it was given untouched', () => {
    const option = create(OptionMessageSchema, { customFps: 24 })
    loginRequest(fields(option), new Uint8Array())
    expect(option.disableAudio).toBe(OptionMessage_BoolOption.NotSet)
    expect(option.supportedDecoding).toBeUndefined()
  })
})
