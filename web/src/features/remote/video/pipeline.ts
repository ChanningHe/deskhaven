import type { VideoFrame as VideoFrameMessage } from '../proto/gen/message_pb.js'
import { encodedFrames } from './codec'
import type { CreateDecoder, FrameDecoder } from './decoder'

export interface VideoPipelineOptions {
  createDecoder: CreateDecoder
  /** A decoded frame; the receiver owns it and must close it. */
  onFrame(frame: VideoFrame): void
  /** Tell the peer one VideoFrame message is consumed (Misc.video_received). */
  ack(): void
  /** Ask the peer for a key frame (Misc.refresh_video). */
  requestKeyFrame(): void
  /** Decoding keeps failing; the pipeline has stopped. */
  onFatal(error: Error): void
}

/**
 * Delta frames are dropped once this many chunks wait in the decoder: it is not keeping up,
 * and queued frames only add delay. Dropping breaks the reference chain, so decoding resumes
 * at the next key frame.
 */
export const MAX_DECODE_QUEUE = 4

/** Failures in a row, with no frame decoded in between, before the pipeline gives up. */
export const MAX_FAILURES = 3

interface Pending {
  /** Timestamp of the message's last chunk: its output means the whole message is done. */
  last: number
}

/**
 * Feeds VideoFrame messages to a decoder. The peer is told `video_received` once per message,
 * when its frame is decoded or the message is dropped, since it waits for that before sending
 * more (rustdesk 1.5.0 src/server/connection.rs:1074-1084, video_service.rs:205-245).
 */
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

  constructor(options: VideoPipelineOptions) {
    this.#options = options
  }

  push(message: VideoFrameMessage): void {
    if (this.#closed) return
    const encoded = encodedFrames(message)
    if (!encoded || encoded.frames.length === 0) {
      this.#options.ack()
      return
    }
    const firstKey = encoded.frames.findIndex((f) => f.key)
    const needsKey = this.#awaitingKey || encoded.codec !== this.#codec
    if (firstKey < 0 && (needsKey || this.#backedUp())) {
      this.#drop()
      return
    }
    const decoder = this.#decoderFor(encoded.codec)
    const frames = encoded.frames.slice(needsKey ? firstKey : 0)
    try {
      for (const frame of frames) {
        decoder.decode({ key: frame.key, timestamp: this.#nextTimestamp++, data: frame.data })
      }
    } catch (error) {
      this.#options.ack()
      this.#fail(error)
      return
    }
    this.#pending.push({ last: this.#nextTimestamp - 1 })
    this.#awaitingKey = false
    this.#keyRequested = false
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
    return (this.#decoder?.decodeQueueSize ?? 0) >= MAX_DECODE_QUEUE
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
    const waiting = this.#pending.filter((p) => p.last > frame.timestamp)
    this.#ack(this.#pending.length - waiting.length)
    this.#pending = waiting
    this.#options.onFrame(frame)
  }

  #drop(): void {
    this.#options.ack()
    this.#awaitKey()
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

  /** In-flight messages will never be decoded now; ack them so the peer is not left waiting. */
  #closeDecoder(): void {
    this.#generation++
    this.#decoder?.close()
    this.#decoder = null
    this.#codec = null
    this.#ack(this.#pending.length)
    this.#pending = []
  }

  #ack(messages: number): void {
    for (let i = 0; i < messages; i++) this.#options.ack()
  }
}
