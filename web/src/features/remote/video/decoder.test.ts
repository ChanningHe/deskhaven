import { afterEach, describe, expect, it, vi } from 'vitest'

import { createWebCodecsDecoder } from './decoder'

class StubDecoder {
  static last: StubDecoder | null = null
  readonly configs: VideoDecoderConfig[] = []
  readonly chunks: unknown[] = []
  state = 'unconfigured'
  decodeQueueSize = 0

  constructor() {
    StubDecoder.last = this
  }

  configure(config: VideoDecoderConfig): void {
    this.configs.push(config)
    this.state = 'configured'
  }

  decode(chunk: unknown): void {
    this.chunks.push(chunk)
  }

  close(): void {
    this.state = 'closed'
  }
}

class StubChunk {
  readonly init: EncodedVideoChunkInit

  constructor(init: EncodedVideoChunkInit) {
    this.init = init
  }
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('createWebCodecsDecoder', () => {
  // WebCodecs AVC and HEVC codec registrations §3: no description means an Annex B bitstream.
  it.each(['avc1.640033', 'hvc1.1.6.L153.B0'])(
    'configures %s with no description, for Annex B',
    (codec) => {
      vi.stubGlobal('VideoDecoder', StubDecoder)
      vi.stubGlobal('EncodedVideoChunk', StubChunk)
      const decoder = createWebCodecsDecoder(codec, { output: vi.fn(), error: vi.fn() })
      const stub = StubDecoder.last
      expect(stub?.configs).toEqual([{ codec, optimizeForLatency: true }])
      expect(stub?.configs[0]).not.toHaveProperty('description')

      decoder.decode({ key: true, timestamp: 0, data: Uint8Array.of(0, 0, 0, 1, 0x67) })
      decoder.decode({ key: false, timestamp: 1, data: Uint8Array.of(0, 0, 0, 1, 0x41) })
      expect(stub?.chunks.map((c) => (c as StubChunk).init.type)).toEqual(['key', 'delta'])
    },
  )
})
