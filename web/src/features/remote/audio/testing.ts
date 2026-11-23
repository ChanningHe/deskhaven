import type { AudioDecoderCallbacks, AudioDecoders, DecodedAudio, OpusDecoder } from './decoder'
import type { AudioFormat } from './format'
import type { AudioOutput, PcmBuffer, ScheduledSound } from './pipeline'
import type { AudioDeps, SpeakerContext } from './speaker'

export class FakeOpusDecoder implements OpusDecoder {
  readonly decoded: Uint8Array[] = []
  readonly timestamps: number[] = []
  closed = false
  refuse = false
  decodeQueueSize = 0
  readonly format: AudioFormat
  readonly #callbacks: AudioDecoderCallbacks

  constructor(format: AudioFormat, callbacks: AudioDecoderCallbacks) {
    this.format = format
    this.#callbacks = callbacks
  }

  decode(data: Uint8Array, timestamp: number): void {
    if (this.refuse) throw new Error('refused')
    this.decoded.push(data)
    this.timestamps.push(timestamp)
  }

  close(): void {
    this.closed = true
  }

  emit(frames = 480): FakeAudioData {
    const data = new FakeAudioData(this.format.channels, frames, this.format.sampleRate)
    this.#callbacks.output(data)
    return data
  }

  fail(): void {
    this.closed = true
    this.#callbacks.error(new Error('corrupt packet'))
  }
}

export class FakeAudioDecoders implements AudioDecoders {
  readonly all: FakeOpusDecoder[] = []
  readonly asked: AudioFormat[] = []
  unsupported: AudioFormat[] = []
  refuse = false
  probeThrows = false

  readonly supports = (format: AudioFormat): Promise<boolean> => {
    this.asked.push(format)
    if (this.probeThrows) return Promise.reject(new Error('probe failed'))
    return Promise.resolve(
      !this.unsupported.some(
        (u) => u.sampleRate === format.sampleRate && u.channels === format.channels,
      ),
    )
  }

  readonly create = (format: AudioFormat, callbacks: AudioDecoderCallbacks): FakeOpusDecoder => {
    if (this.refuse) throw new Error('no Opus here')
    const decoder = new FakeOpusDecoder(format, callbacks)
    this.all.push(decoder)
    return decoder
  }

  get last(): FakeOpusDecoder {
    const decoder = this.all.at(-1)
    if (!decoder) throw new Error('no decoder yet')
    return decoder
  }
}

export class FakeAudioData implements DecodedAudio {
  readonly numberOfChannels: number
  readonly numberOfFrames: number
  readonly sampleRate: number
  closed = false
  readonly copied: number[] = []

  constructor(channels: number, frames: number, sampleRate: number) {
    this.numberOfChannels = channels
    this.numberOfFrames = frames
    this.sampleRate = sampleRate
  }

  copyTo(destination: AllowSharedBufferSource, options: AudioDataCopyToOptions): void {
    if (!(destination instanceof Float32Array)) throw new Error('not a Float32Array')
    if (options.format !== 'f32-planar') throw new Error('not planar f32')
    destination.fill(options.planeIndex + 1)
    this.copied.push(options.planeIndex)
  }

  close(): void {
    this.closed = true
  }
}

class FakeBuffer implements PcmBuffer {
  readonly duration: number
  readonly channels: Float32Array[]

  constructor(channels: number, frames: number, sampleRate: number) {
    this.duration = frames / sampleRate
    this.channels = Array.from({ length: channels }, () => new Float32Array(frames))
  }

  getChannelData(channel: number): Float32Array {
    const data = this.channels[channel]
    if (!data) throw new Error(`no channel ${String(channel)}`)
    return data
  }
}

export interface FakeSound {
  buffer: FakeBuffer
  when: number
  stopped: boolean
  ended: () => void
}

export class FakeOutput implements AudioOutput {
  running = true
  currentTime = 10
  refuse = false
  readonly sounds: FakeSound[] = []

  createBuffer(channels: number, frames: number, sampleRate: number): FakeBuffer {
    if (this.refuse) throw new DOMException('unsupported sample rate', 'NotSupportedError')
    return new FakeBuffer(channels, frames, sampleRate)
  }

  schedule(buffer: PcmBuffer, when: number, onEnded: () => void): ScheduledSound {
    if (!(buffer instanceof FakeBuffer)) throw new Error('not a buffer of this output')
    const sound: FakeSound = { buffer, when, stopped: false, ended: onEnded }
    this.sounds.push(sound)
    return {
      stop: () => {
        sound.stopped = true
        onEnded()
      },
    }
  }

  playing(): [number, number][] {
    return this.sounds
      .filter((s) => !s.stopped)
      .map((s) => [ms(s.when), ms(s.when + s.buffer.duration)])
  }
}

function ms(seconds: number): number {
  return Math.round(seconds * 1000 * 100) / 100
}

export class FakeContext {
  state: AudioContextState = 'suspended'
  currentTime = 0
  resumed = 0
  onstatechange: (() => void) | null = null
  readonly destination = {}
  #stopThrows = false

  createBuffer(channels: number, frames: number, sampleRate: number): FakeBuffer {
    return new FakeBuffer(channels, frames, sampleRate)
  }

  createBufferSource() {
    return {
      buffer: null as unknown,
      onended: null as unknown,
      connect: () => undefined,
      start: () => undefined,
      stop: () => {
        if (this.#stopThrows) throw new DOMException('not started', 'InvalidStateError')
      },
    }
  }

  refuseStop(): void {
    this.#stopThrows = true
  }

  resume(): Promise<void> {
    this.resumed++
    this.browserSets('running')
    return Promise.resolve()
  }

  close(): Promise<void> {
    this.browserSets('closed')
    return Promise.resolve()
  }

  browserSets(state: AudioContextState): void {
    this.state = state
    this.onstatechange?.()
  }
}

export function fakeAudioDeps({
  opus = true,
  context = 'works',
}: { opus?: boolean | 'throws'; context?: 'works' | 'throws' } = {}) {
  const contexts: FakeContext[] = []
  const deps: AudioDeps = {
    decodesOpus: () =>
      opus === 'throws' ? Promise.reject(new Error('probe failed')) : Promise.resolve(opus),
    createContext: () => {
      if (context === 'throws') throw new Error('too many AudioContexts')
      const created = new FakeContext()
      contexts.push(created)
      return created as unknown as SpeakerContext
    },
  }
  return { deps, contexts }
}
