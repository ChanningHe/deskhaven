import { create } from '@bufbuild/protobuf'
import { vi } from 'vitest'

import {
  EncodedVideoFrameSchema,
  EncodedVideoFramesSchema,
  VideoFrameSchema,
  type VideoFrame as VideoFrameMessage,
} from '../proto/gen/message_pb.js'
import type { CreateDecoder, DecoderCallbacks, EncodedChunk, FrameDecoder } from './decoder'

export { fixtureKeyFrame } from './fixture-frame'

/** A synthetic frame; the fake decoder never looks inside. */
export function vp9Frame(
  key: boolean,
  data = Uint8Array.of(0x86, 0, key ? 1 : 0),
): VideoFrameMessage {
  return create(VideoFrameSchema, {
    union: {
      case: 'vp9s',
      value: create(EncodedVideoFramesSchema, {
        frames: [create(EncodedVideoFrameSchema, { key, data })],
      }),
    },
  })
}

export interface FakeFrame {
  timestamp: number
  close: () => void
}

export class FakeDecoder implements FrameDecoder {
  decodeQueueSize = 0
  readonly decoded: EncodedChunk[] = []
  closed = false
  throwOnDecode: Error | null = null
  readonly codec: string
  readonly #callbacks: DecoderCallbacks

  constructor(codec: string, callbacks: DecoderCallbacks) {
    this.codec = codec
    this.#callbacks = callbacks
  }

  decode(chunk: EncodedChunk): void {
    if (this.throwOnDecode) throw this.throwOnDecode
    this.decoded.push(chunk)
  }

  close(): void {
    this.closed = true
  }

  /** Emits the frame decoded from the chunk at `index` of what this decoder took. */
  output(index = this.decoded.length - 1): FakeFrame {
    const chunk = this.decoded[index]
    if (!chunk) throw new Error(`no chunk ${index}`)
    const frame = { timestamp: chunk.timestamp, close: vi.fn() }
    this.#callbacks.output(frame as unknown as VideoFrame)
    return frame
  }

  fail(error = new Error('decode failed')): void {
    this.closed = true
    this.#callbacks.error(error)
  }
}

export class FakeDecoders {
  readonly created: FakeDecoder[] = []

  readonly create: CreateDecoder = (codec, callbacks) => {
    const decoder = new FakeDecoder(codec, callbacks)
    this.created.push(decoder)
    return decoder
  }

  get last(): FakeDecoder {
    const decoder = this.created.at(-1)
    if (!decoder) throw new Error('no decoder created')
    return decoder
  }
}
