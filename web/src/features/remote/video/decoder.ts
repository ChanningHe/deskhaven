/** One encoded frame; `timestamp` comes back on the decoded VideoFrame. */
export interface EncodedChunk {
  key: boolean
  timestamp: number
  data: Uint8Array
}

export interface DecoderCallbacks {
  output(frame: VideoFrame): void
  error(error: Error): void
}

export interface FrameDecoder {
  readonly decodeQueueSize: number
  /** Throws when the decoder refuses the chunk (for one, a delta frame after configure). */
  decode(chunk: EncodedChunk): void
  close(): void
}

export type CreateDecoder = (codec: string, callbacks: DecoderCallbacks) => FrameDecoder

export const createWebCodecsDecoder: CreateDecoder = (codec, callbacks) => {
  const decoder = new VideoDecoder({
    output: (frame) => {
      callbacks.output(frame)
    },
    error: (error) => {
      callbacks.error(error)
    },
  })
  decoder.configure({ codec, optimizeForLatency: true })
  return {
    get decodeQueueSize() {
      return decoder.decodeQueueSize
    },
    decode(chunk) {
      decoder.decode(
        new EncodedVideoChunk({
          type: chunk.key ? 'key' : 'delta',
          timestamp: chunk.timestamp,
          data: chunk.data,
        }),
      )
    },
    close() {
      if (decoder.state !== 'closed') decoder.close()
    },
  }
}
