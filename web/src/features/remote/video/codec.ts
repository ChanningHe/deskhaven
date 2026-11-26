import {
  DECODER_PROBES,
  EXPERIMENTAL_CODECS,
  LOGIN_CODECS,
  VIDEO_CODECS,
  decodable,
  preferring,
  probeDecoders,
  supportedDecoding,
  type DecoderSupport,
  type VideoCodec,
} from '../handshake/login'
import type {
  EncodedVideoFrame,
  SupportedDecoding,
  SupportedEncoding,
  VideoFrame as VideoFrameMessage,
} from '../proto/gen/message_pb.js'

/**
 * WebCodecs codec strings (WebCodecs Codec Registry), the same ones probed before login, so we
 * decode only what we declared. What the peer encodes when we declare none of i444 or 10-bit:
 * - VP9 `vp09.00.10.08`: profile 0 (4:2:0), level 1.0, 8 bits (VP9 ISO-BMFF binding §codecs).
 *   Decoders do not hold a stream to the level; the 1280×800 capture decodes under it.
 * - AV1 `av01.0.04M.08`: Main profile, level 3.0, Main tier, 8 bits (AV1 ISO-BMFF binding).
 * - VP8 `vp8`: the registry's only VP8 string.
 * - H264 `avc1.640033` and H265 `hvc1.1.6.L153.B0`: High and Main profile at level 5.1, configured
 *   with no description, so chunks are Annex B with in-band parameter sets.
 */
export function webCodecsString(codec: VideoCodec): string {
  return DECODER_PROBES[codec]
}

export const CODEC_NAMES: Readonly<Record<VideoCodec, string>> = {
  vp9: 'VP9',
  av1: 'AV1',
  vp8: 'VP8',
  h264: 'H264',
  h265: 'H265',
}

const UNION_CODECS = {
  vp9s: 'vp9',
  av1s: 'av1',
  vp8s: 'vp8',
  h264s: 'h264',
  h265s: 'h265',
} as const

export interface EncodedFrames {
  codec: VideoCodec
  frames: readonly EncodedVideoFrame[]
}

export function encodedFrames(message: VideoFrameMessage): EncodedFrames | null {
  const { union } = message
  switch (union.case) {
    case 'vp9s':
    case 'av1s':
    case 'vp8s':
    case 'h264s':
    case 'h265s':
      return { codec: UNION_CODECS[union.case], frames: union.value.frames }
    default:
      return null
  }
}

export async function isCodecSupported(codec: string): Promise<boolean> {
  if (typeof VideoDecoder === 'undefined') return false
  try {
    const result = await VideoDecoder.isConfigSupported({ codec, optimizeForLatency: true })
    return result.supported === true
  } catch {
    return false
  }
}

export function probeVideoDecoders(): Promise<DecoderSupport> {
  return probeDecoders(isCodecSupported)
}

export interface CodecEvents {
  accepts(codec: VideoCodec): boolean
  received(codec: VideoCodec): void
  decoded(codec: VideoCodec): void
  failed(codec: VideoCodec): void
}

/** Given up in this session: its frames did not decode (`failed`) or the peer sent another (`ignored`). */
export type CodecSupport = 'ok' | 'browser' | 'peer' | 'failed' | 'ignored'

export interface CodecView {
  support: Readonly<Record<VideoCodec, CodecSupport>>
  /** What we last asked the peer to encode: the login's codec (VP9) until a switch. */
  requested: VideoCodec
  receiving: VideoCodec | null
  failed?: VideoCodec
  failedAt?: number
}

/**
 * The peer restarts its video service on a codec change and waits up to 1 s after it, then encodes
 * a key frame (AV1 must manage one at 1080p within 90 ms). Five times that leaves room for a slow
 * link and a key frame request.
 */
export const SWITCH_TIMEOUT_MS = 5000

/** Below the pipeline's MAX_FAILURES, which ends the session; the official client gives up after 3. */
export const SWITCH_ERRORS = 2

export interface CodecSwitchOptions {
  support: DecoderSupport
  now(): number
  /** Sends Misc.option with only supported_decoding set. */
  send(decoding: SupportedDecoding): void
  refresh(): void
  onChange(view: CodecView): void
}

interface Watch {
  codec: VideoCodec
  /** What arrived before the request: its frames say nothing about the new codec. */
  from: VideoCodec
  since: number
  /**
   * Another codec still came in the second half of the wait: the peer goes on with it. Earlier
   * frames may have been in flight, and a still screen after the encoder restart sends none.
   */
  late: boolean
  arrived: boolean
}

const NO_PEER_CODECS: Readonly<Record<VideoCodec, boolean>> = {
  vp9: true,
  av1: false,
  vp8: false,
  h264: false,
  h265: false,
}

/**
 * Falls back to VP9 when the picked codec's frames do not come out of the decoder while the peer
 * is sending, and withdraws it for the rest of the session. A silent peer is asked for a refresh,
 * not judged.
 */
export class CodecSwitch implements CodecEvents {
  readonly #options: CodecSwitchOptions
  #peer = NO_PEER_CODECS
  readonly #given = new Map<VideoCodec, 'failed' | 'ignored'>()
  readonly #login: VideoCodec
  #wanted: VideoCodec = 'vp9'
  #watch: Watch | null = null
  #errors = 0
  #view: CodecView

  constructor(options: CodecSwitchOptions) {
    this.#options = options
    this.#login = decodable(options.support, LOGIN_CODECS)[0] ?? 'vp9'
    this.#view = { support: this.#support(), requested: this.#login, receiving: null }
  }

  get view(): CodecView {
    return this.#view
  }

  /** PeerInfo.encoding: what the peer can encode. */
  peerEncodes(encoding: SupportedEncoding | undefined): void {
    this.#peerCan({
      vp9: true,
      av1: encoding?.av1 === true,
      vp8: encoding?.vp8 === true,
      h264: encoding?.h264 === true,
      h265: encoding?.h265 === true,
    })
  }

  /**
   * H264 and H265 stay as PeerInfo said: we declare them only once asked for, so this says nothing
   * about them before.
   */
  peerUsable(encoding: SupportedEncoding): void {
    this.#peerCan({ ...this.#peer, av1: encoding.av1, vp8: encoding.vp8 })
  }

  want(codec: VideoCodec): void {
    this.#wanted = codec
    this.#apply()
  }

  accepts(codec: VideoCodec): boolean {
    return this.#options.support[codec] && !this.#given.has(codec)
  }

  received(codec: VideoCodec): void {
    const watch = this.#watch
    if (watch) {
      if (codec === watch.codec) watch.arrived = true
      else this.#sawOther(watch)
    }
    if (codec !== this.#view.receiving) this.#update({ receiving: codec })
  }

  decoded(codec: VideoCodec): void {
    if (codec === this.#view.requested) this.#errors = 0
    const watch = this.#watch
    if (!watch) return
    // Asked for H264/H265, a third codec is the peer's Auto pick for a codec not every connection
    // declared: a refusal, like the old codec.
    const elsewhere = codec !== watch.from && !EXPERIMENTAL_CODECS.includes(watch.codec)
    if (codec === watch.codec || elsewhere) this.#watch = null
    else this.#sawOther(watch)
  }

  failed(codec: VideoCodec): void {
    if (codec !== this.#view.requested || codec === this.#login) return
    if (++this.#errors >= SWITCH_ERRORS) this.#fallBack(codec, 'failed')
  }

  /** Called about once a second. */
  tick(): void {
    const watch = this.#watch
    const now = this.#options.now()
    if (!watch || now - watch.since < SWITCH_TIMEOUT_MS) return
    if (watch.arrived || watch.late) {
      this.#fallBack(watch.codec, watch.arrived ? 'failed' : 'ignored')
      return
    }
    watch.since = now
    this.#options.refresh()
  }

  #peerCan(peer: Record<VideoCodec, boolean>): void {
    this.#peer = peer
    if (this.#watch && !this.#peer[this.#watch.codec]) this.#watch = null
    this.#update({ support: this.#support() })
    this.#apply()
  }

  #sawOther(watch: Watch): void {
    if (this.#options.now() - watch.since >= SWITCH_TIMEOUT_MS / 2) watch.late = true
  }

  #support(): Record<VideoCodec, CodecSupport> {
    const of = (codec: VideoCodec): CodecSupport => {
      if (!this.#peer[codec]) return 'peer'
      if (!this.#options.support[codec]) return 'browser'
      return this.#given.get(codec) ?? 'ok'
    }
    return {
      vp9: of('vp9'),
      av1: of('av1'),
      vp8: of('vp8'),
      h264: of('h264'),
      h265: of('h265'),
    }
  }

  #apply(): void {
    const codec = this.#wanted
    if (codec === this.#view.requested || this.#view.support[codec] !== 'ok') return
    this.#request(codec)
  }

  /** Sends first and tells after: a listener that asks again then finds it already asked. */
  #request(codec: VideoCodec, patch: Partial<CodecView> = {}): void {
    const declared = VIDEO_CODECS.filter(
      (c) => !this.#given.has(c) && (c === codec || LOGIN_CODECS.includes(c)),
    )
    const decoding = supportedDecoding(this.#options.support, declared)
    if (!decoding) return
    const from = this.#view.receiving ?? this.#view.requested
    const since = this.#options.now()
    this.#errors = 0
    this.#watch = codec === this.#login ? null : { codec, from, since, late: false, arrived: false }
    this.#options.send(preferring(decoding, codec))
    this.#update({ ...patch, requested: codec })
  }

  #fallBack(codec: VideoCodec, why: 'failed' | 'ignored'): void {
    this.#given.set(codec, why)
    this.#wanted = this.#login
    this.#request(this.#login, {
      support: this.#support(),
      failed: codec,
      failedAt: this.#options.now(),
    })
  }

  #update(patch: Partial<CodecView>): void {
    const next = { ...this.#view, ...patch }
    if (sameView(next, this.#view)) return
    this.#view = next
    this.#options.onChange(next)
  }
}

function sameView(a: CodecView, b: CodecView): boolean {
  return (
    a.requested === b.requested &&
    a.receiving === b.receiving &&
    a.failed === b.failed &&
    a.failedAt === b.failedAt &&
    VIDEO_CODECS.every((codec) => a.support[codec] === b.support[codec])
  )
}
