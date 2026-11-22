import { DECODER_PROBES, probeDecoders, type DecoderSupport } from '../handshake/login'
import type { EncodedVideoFrame, VideoFrame as VideoFrameMessage } from '../proto/gen/message_pb.js'

/**
 * WebCodecs codec strings (WebCodecs Codec Registry), the same ones probed before login, so we
 * decode only what we declared. What the peer encodes when we declare none of i444 or 10-bit:
 * - VP9 `vp09.00.10.08`: profile 0 (4:2:0), level 1.0, 8 bits (VP9 ISO-BMFF binding §codecs).
 *   Decoders do not hold a stream to the level; the 1280×800 capture decodes under it.
 * - AV1 `av01.0.04M.08`: Main profile, level 3.0, Main tier, 8 bits (AV1 ISO-BMFF binding).
 * - VP8 `vp8`: the registry's only VP8 string.
 */
const CODECS = {
  vp9s: DECODER_PROBES.vp9,
  av1s: DECODER_PROBES.av1,
  vp8s: DECODER_PROBES.vp8,
} as const

export interface EncodedFrames {
  codec: string
  frames: readonly EncodedVideoFrame[]
}

/** The codec and frames of a VideoFrame message, or null for an encoding we do not decode. */
export function encodedFrames(message: VideoFrameMessage): EncodedFrames | null {
  const { union } = message
  switch (union.case) {
    case 'vp9s':
    case 'av1s':
    case 'vp8s':
      return { codec: CODECS[union.case], frames: union.value.frames }
    default:
      return null
  }
}

/** Whether this browser decodes `codec`; false where WebCodecs is missing or refuses it. */
export async function isCodecSupported(codec: string): Promise<boolean> {
  if (typeof VideoDecoder === 'undefined') return false
  try {
    const result = await VideoDecoder.isConfigSupported({ codec, optimizeForLatency: true })
    return result.supported === true
  } catch {
    return false
  }
}

/** What to declare in LoginRequest.option.supported_decoding. */
export function probeVideoDecoders(): Promise<DecoderSupport> {
  return probeDecoders(isCodecSupported)
}
