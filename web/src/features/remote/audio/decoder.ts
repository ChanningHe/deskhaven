import type { AudioFormat } from './format'

export type DecodedAudio = Pick<
  AudioData,
  'numberOfChannels' | 'numberOfFrames' | 'sampleRate' | 'copyTo' | 'close'
>

export interface AudioDecoderCallbacks {
  output(data: DecodedAudio): void
  error(error: Error): void
}

export interface OpusDecoder {
  readonly decodeQueueSize: number
  decode(data: Uint8Array, timestamp: number): void
  close(): void
}

export interface AudioDecoders {
  create(format: AudioFormat, callbacks: AudioDecoderCallbacks): OpusDecoder
  supports(format: AudioFormat): Promise<boolean>
}

function opusConfig({ sampleRate, channels }: AudioFormat): AudioDecoderConfig {
  return { codec: 'opus', sampleRate, numberOfChannels: channels }
}

function createWebCodecsAudioDecoder(
  format: AudioFormat,
  callbacks: AudioDecoderCallbacks,
): OpusDecoder {
  const decoder = new AudioDecoder({
    output: (data) => {
      callbacks.output(data)
    },
    error: (error) => {
      callbacks.error(error)
    },
  })
  try {
    decoder.configure(opusConfig(format))
  } catch (error) {
    decoder.close()
    throw error
  }
  return {
    get decodeQueueSize() {
      return decoder.decodeQueueSize
    },
    decode(data, timestamp) {
      decoder.decode(new EncodedAudioChunk({ type: 'key', timestamp, data }))
    },
    close() {
      if (decoder.state !== 'closed') decoder.close()
    },
  }
}

async function webCodecsSupports(format: AudioFormat): Promise<boolean> {
  if (typeof AudioDecoder === 'undefined') return false
  try {
    const { supported } = await AudioDecoder.isConfigSupported(opusConfig(format))
    return supported === true
  } catch {
    return false
  }
}

export const webCodecsAudio: AudioDecoders = {
  create: createWebCodecsAudioDecoder,
  supports: webCodecsSupports,
}

// The peer encodes stereo at 48 kHz.
export function webCodecsDecodesOpus(): Promise<boolean> {
  return webCodecsSupports({ sampleRate: 48000, channels: 2 })
}
