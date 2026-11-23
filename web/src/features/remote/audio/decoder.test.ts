import { afterEach, describe, expect, it, vi } from 'vitest'

import { webCodecsAudio } from './decoder'

class RefusingDecoder {
  static made: RefusingDecoder[] = []
  state = 'unconfigured'
  closed = 0

  constructor() {
    RefusingDecoder.made.push(this)
  }

  configure(): void {
    throw new DOMException('no such config', 'NotSupportedError')
  }

  close(): void {
    this.closed++
    this.state = 'closed'
  }
}

afterEach(() => {
  vi.unstubAllGlobals()
  RefusingDecoder.made = []
})

describe('webCodecsAudio.create', () => {
  it('closes the decoder it made when configure throws', () => {
    vi.stubGlobal('AudioDecoder', RefusingDecoder)
    const callbacks = { output: () => undefined, error: () => undefined }
    expect(() => webCodecsAudio.create({ sampleRate: 48000, channels: 2 }, callbacks)).toThrow(
      'no such config',
    )
    expect(RefusingDecoder.made.map((d) => d.closed)).toEqual([1])
  })
})

describe('webCodecsAudio.supports', () => {
  it('asks about the exact rate and channels', async () => {
    const isConfigSupported = vi.fn(() => Promise.resolve({ supported: false }))
    vi.stubGlobal('AudioDecoder', { isConfigSupported })
    expect(await webCodecsAudio.supports({ sampleRate: 24000, channels: 1 })).toBe(false)
    expect(isConfigSupported).toHaveBeenCalledWith({
      codec: 'opus',
      sampleRate: 24000,
      numberOfChannels: 1,
    })
  })

  it('is false without WebCodecs or when the browser throws', async () => {
    vi.stubGlobal('AudioDecoder', undefined)
    expect(await webCodecsAudio.supports({ sampleRate: 48000, channels: 2 })).toBe(false)
    vi.stubGlobal('AudioDecoder', {
      isConfigSupported: () => Promise.reject(new TypeError('bad config')),
    })
    expect(await webCodecsAudio.supports({ sampleRate: 48000, channels: 2 })).toBe(false)
  })
})
