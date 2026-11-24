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

/** Protocol version we claim; the message.proto this client is generated from is the 1.5.0 tag. */
export const CLIENT_VERSION = '1.5.0'
export const CLIENT_PLATFORM = 'Web'

export type VideoCodec = 'vp9' | 'av1' | 'vp8'

/** In the order we prefer them at login. */
export const VIDEO_CODECS: readonly VideoCodec[] = ['vp9', 'av1', 'vp8']

export type DecoderSupport = Record<VideoCodec, boolean>

/** Codec strings probed with VideoDecoder.isConfigSupported, one per codec we can declare. */
export const DECODER_PROBES: Readonly<Record<VideoCodec, string>> = {
  vp9: 'vp09.00.10.08',
  av1: 'av01.0.04M.08',
  vp8: 'vp8',
}

export async function probeDecoders(
  isSupported: (codec: string) => Promise<boolean>,
): Promise<DecoderSupport> {
  const [vp9, av1, vp8] = await Promise.all([
    isSupported(DECODER_PROBES.vp9),
    isSupported(DECODER_PROBES.av1),
    isSupported(DECODER_PROBES.vp8),
  ])
  return { vp9, av1, vp8 }
}

const PREFER: Readonly<Record<VideoCodec, SupportedDecoding_PreferCodec>> = {
  vp9: SupportedDecoding_PreferCodec.VP9,
  av1: SupportedDecoding_PreferCodec.AV1,
  vp8: SupportedDecoding_PreferCodec.VP8,
}

/** The codecs of `declared` this browser decodes, in VIDEO_CODECS order. */
export function decodable(support: DecoderSupport, declared: readonly VideoCodec[]): VideoCodec[] {
  return VIDEO_CODECS.filter((codec) => support[codec] && declared.includes(codec))
}

/**
 * The abilities to declare: the decodable codecs of `declared`, preferring VP9 (AV1, then VP8,
 * where VP9 is not decodable). Null when none is decodable.
 */
export function supportedDecoding(
  support: DecoderSupport,
  declared: readonly VideoCodec[],
): SupportedDecoding | null {
  const able = decodable(support, declared)
  const prefer = able[0]
  if (prefer === undefined) return null
  return create(SupportedDecodingSchema, {
    abilityVp9: able.includes('vp9') ? 1 : 0,
    abilityAv1: able.includes('av1') ? 1 : 0,
    abilityVp8: able.includes('vp8') ? 1 : 0,
    prefer: PREFER[prefer],
  })
}

/** `decoding` asking the peer to encode `codec`. */
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

// LoginResponse.error strings the peer sends (rustdesk 1.5.0 src/client.rs:114-118, emitted
// by src/server/connection.rs). Anything else ends the session, as in handle_login_error.
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
