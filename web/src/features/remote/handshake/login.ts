import { clone, create } from '@bufbuild/protobuf'

import {
  LoginRequestSchema,
  MessageSchema,
  OptionMessageSchema,
  SupportedDecoding_PreferCodec,
  SupportedDecodingSchema,
  type Message,
  type OptionMessage,
  type PeerInfo,
  type SupportedDecoding,
} from '../proto/gen/message_pb.js'

/** Protocol version we claim: message.proto is generated from the 1.5.0 tag. */
export const CLIENT_VERSION = '1.5.0'
export const CLIENT_PLATFORM = 'Web'

export type VideoCodec = 'vp9' | 'av1' | 'vp8' | 'h264' | 'h265'

export const LOGIN_CODECS: readonly VideoCodec[] = ['vp9', 'av1', 'vp8']

/**
 * Declared only while picked: a peer encodes them in hardware alone, and once every connection
 * declares one, its Auto choice takes H265 > H264 over the rest for all of them.
 */
export const EXPERIMENTAL_CODECS: readonly VideoCodec[] = ['h264', 'h265']

export const VIDEO_CODECS: readonly VideoCodec[] = [...LOGIN_CODECS, ...EXPERIMENTAL_CODECS]

export type DecoderSupport = Record<VideoCodec, boolean>

/** H264 High 5.1 and H265 Main 5.1 cover a 4K screen; with no description the decoder takes Annex B. */
export const DECODER_PROBES: Readonly<Record<VideoCodec, string>> = {
  vp9: 'vp09.00.10.08',
  av1: 'av01.0.04M.08',
  vp8: 'vp8',
  h264: 'avc1.640033',
  h265: 'hvc1.1.6.L153.B0',
}

export async function probeDecoders(
  isSupported: (codec: string) => Promise<boolean>,
): Promise<DecoderSupport> {
  const supported = await Promise.all(
    VIDEO_CODECS.map((codec) => isSupported(DECODER_PROBES[codec])),
  )
  const support: DecoderSupport = { vp9: false, av1: false, vp8: false, h264: false, h265: false }
  VIDEO_CODECS.forEach((codec, i) => {
    support[codec] = supported[i] === true
  })
  return support
}

const PREFER: Readonly<Record<VideoCodec, SupportedDecoding_PreferCodec>> = {
  vp9: SupportedDecoding_PreferCodec.VP9,
  av1: SupportedDecoding_PreferCodec.AV1,
  vp8: SupportedDecoding_PreferCodec.VP8,
  h264: SupportedDecoding_PreferCodec.H264,
  h265: SupportedDecoding_PreferCodec.H265,
}

export function decodable(support: DecoderSupport, declared: readonly VideoCodec[]): VideoCodec[] {
  return VIDEO_CODECS.filter((codec) => support[codec] && declared.includes(codec))
}

export function supportedDecoding(
  support: DecoderSupport,
  declared: readonly VideoCodec[],
): SupportedDecoding | null {
  const able = decodable(support, declared)
  const prefer = able[0]
  if (prefer === undefined) return null
  const ability = (codec: VideoCodec) => (able.includes(codec) ? 1 : 0)
  return create(SupportedDecodingSchema, {
    abilityVp9: ability('vp9'),
    abilityAv1: ability('av1'),
    abilityVp8: ability('vp8'),
    abilityH264: ability('h264'),
    abilityH265: ability('h265'),
    prefer: PREFER[prefer],
  })
}

export function preferring(decoding: SupportedDecoding, codec: VideoCodec): SupportedDecoding {
  const copy = clone(SupportedDecodingSchema, decoding)
  copy.prefer = PREFER[codec]
  return copy
}

export interface LoginFields {
  targetId: string
  myId: string
  myName: string
  sessionId: bigint
  decoding: SupportedDecoding
  option: OptionMessage
}

/** `password` is the login proof, or empty to ask the peer's user to accept the session. */
export function loginRequest(fields: LoginFields, password: Uint8Array): Message {
  const option = clone(OptionMessageSchema, fields.option)
  option.supportedDecoding = fields.decoding
  return create(MessageSchema, {
    union: {
      case: 'loginRequest',
      value: create(LoginRequestSchema, {
        username: fields.targetId,
        password,
        myId: fields.myId,
        myName: fields.myName,
        myPlatform: CLIENT_PLATFORM,
        version: CLIENT_VERSION,
        sessionId: fields.sessionId,
        option,
      }),
    },
  })
}

export type LoginState =
  | { kind: 'connecting' }
  | { kind: 'need_password' }
  | { kind: 'wrong_password' }
  | { kind: 'awaiting_approval' }
  | { kind: 'need_2fa'; wrongCode: boolean }
  | { kind: 'active'; peerInfo: PeerInfo }
  | { kind: 'failed'; reason: string }

// LoginResponse.error strings the peer sends; anything else ends the session.
const LOGIN_ERRORS: ReadonlyMap<string, LoginState> = new Map<string, LoginState>([
  ['Empty Password', { kind: 'need_password' }],
  ['Wrong Password', { kind: 'wrong_password' }],
  ['No Password Access', { kind: 'awaiting_approval' }],
  ['2FA Required', { kind: 'need_2fa', wrongCode: false }],
  ['Wrong 2FA Code', { kind: 'need_2fa', wrongCode: true }],
])

export function loginStateForError(error: string): LoginState {
  return LOGIN_ERRORS.get(error) ?? { kind: 'failed', reason: error }
}
