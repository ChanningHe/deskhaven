import type { VideoFrame as VideoFrameMessage } from '../proto/gen/message_pb.js'
import { encodedFrames } from './codec'
import type { CreateDecoder, FrameDecoder } from './decoder'

export interface VideoPipelineOptions {
  createDecoder: CreateDecoder
  /** A decoded frame; the receiver owns it and must close it. */
  onFrame(frame: VideoFrame): void
  /** Ask the peer for a key frame (Misc.refresh_video). */
  requestKeyFrame(): void
  mayRefresh(): boolean
  /** Decoding keeps failing; the pipeline has stopped. */
  onFatal(error: Error): void
  // Reports only: nothing the pipeline decides reads it.
  meter?: VideoMeter
}

export interface VideoMeter {
  now(): number
  received(bytes: number): void
  decoded(latencyMs: number, serviceMs: number): void
}

/**
 * Delta frames may be dropped once this many chunks wait in the decoder: it is not keeping up,
 * and queued frames only add delay. Dropping breaks the reference chain, so decoding resumes
 * at the next key frame.
 */
export const MAX_DECODE_QUEUE = 4

/** Failures in a row, with no frame decoded in between, before the pipeline gives up. */
export const MAX_FAILURES = 3

interface Pending {
  /** Timestamp of the message's last chunk: its output means the whole message is done. */
  last: number
  pushedAt: number
}

export class VideoPipeline {
  readonly #options: VideoPipelineOptions
  #decoder: FrameDecoder | null = null
  #codec: string | null = null
  #generation = 0
  #awaitingKey = true
  #keyRequested = false
  #nextTimestamp = 0
  #pending: Pending[] = []
  #failures = 0
  #closed = false
  #lastOutputAt = 0

  constructor(options: VideoPipelineOptions) {
    this.#options = options
  }

  push(message: VideoFrameMessage): void {
    if (this.#closed) return
    const encoded = encodedFrames(message)
    const meter = this.#options.meter
    meter?.received(encoded?.frames.reduce((sum, f) => sum + f.data.length, 0) ?? 0)
    if (!encoded || encoded.frames.length === 0) return
    const firstKey = encoded.frames.findIndex((f) => f.key)
    const needsKey = this.#awaitingKey || encoded.codec !== this.#codec
    if (firstKey < 0 && (needsKey || this.#backedUp())) {
      this.#awaitKey()
      return
    }
    const decoder = this.#decoderFor(encoded.codec)
    const frames = encoded.frames.slice(needsKey ? firstKey : 0)
    try {
      for (const frame of frames) {
        decoder.decode({ key: frame.key, timestamp: this.#nextTimestamp++, data: frame.data })
      }
    } catch (error) {
      this.#fail(error)
      return
    }
    this.#pending.push({ last: this.#nextTimestamp - 1, pushedAt: meter?.now() ?? 0 })
    this.#awaitingKey = false
    this.#keyRequested = false
  }

  get codec(): string | null {
    return this.#codec
  }

  get decodeQueueSize(): number {
    return this.#decoder?.decodeQueueSize ?? 0
  }

  get queued(): number {
    return Math.max(0, this.#pending.length - 1)
  }

  /** The peer switched display: what is in flight belongs to the old one. */
  reset(): void {
    this.#closeDecoder()
    this.#awaitingKey = true
    this.#keyRequested = false
  }

  close(): void {
    this.#closed = true
    this.#closeDecoder()
  }

  #backedUp(): boolean {
    return this.decodeQueueSize >= MAX_DECODE_QUEUE && this.#options.mayRefresh()
  }

  #decoderFor(codec: string): FrameDecoder {
    if (this.#decoder && this.#codec === codec) return this.#decoder
    this.#closeDecoder()
    const generation = ++this.#generation
    this.#codec = codec
    this.#decoder = this.#options.createDecoder(codec, {
      output: (frame) => {
        this.#onOutput(generation, frame)
      },
      error: (error) => {
        if (generation === this.#generation) this.#fail(error)
      },
    })
    return this.#decoder
  }

  #onOutput(generation: number, frame: VideoFrame): void {
    if (this.#closed || generation !== this.#generation) {
      frame.close()
      return
    }
    this.#failures = 0
    const done = this.#pending.find((p) => p.last === frame.timestamp)
    const meter = this.#options.meter
    if (done && meter) {
      const now = meter.now()
      meter.decoded(now - done.pushedAt, now - Math.max(done.pushedAt, this.#lastOutputAt))
      this.#lastOutputAt = now
    }
    this.#pending = this.#pending.filter((p) => p.last > frame.timestamp)
    this.#options.onFrame(frame)
  }

  #awaitKey(): void {
    this.#awaitingKey = true
    if (this.#keyRequested) return
    this.#keyRequested = true
    this.#options.requestKeyFrame()
  }

  #fail(cause: unknown): void {
    const error = cause instanceof Error ? cause : new Error(String(cause))
    this.#closeDecoder()
    if (++this.#failures >= MAX_FAILURES) {
      this.close()
      this.#options.onFatal(error)
      return
    }
    this.#awaitKey()
  }

  #closeDecoder(): void {
    this.#generation++
    this.#decoder?.close()
    this.#decoder = null
    this.#codec = null
    this.#pending = []
  }
}
