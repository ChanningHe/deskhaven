import type { Message } from '../proto/gen/message_pb.js'
import { ValueStore, type Store } from '../session/store'
import type { AudioDecoders, DecodedAudio, OpusDecoder } from './decoder'
import { acceptsFrame, audioFormat, type AudioFormat } from './format'

export const MAX_DECODE_FAILURES = 5
// Design ruling 「音频」: past 200 ms queued, sound lags the picture; drop rather than drift.
export const MAX_BACKLOG_S = 0.2
// Packets come every 10 ms with network jitter; starting right at `now` would run dry at once.
export const START_LEAD_S = 0.05
// About 200 ms of 10 ms packets: more waiting than that is sound the backlog rule would drop anyway.
export const MAX_DECODE_QUEUE = 20
// A peer alternating bad and good packets, or formats, must not rebuild decoders without end.
export const MAX_DECODER_REBUILDS = 3
export const REBUILD_WINDOW_MS = 1000
const PACKET_US = 10_000

export type AudioState = 'ok' | 'failed' | 'unsupported'

export interface PcmBuffer {
  readonly duration: number
  getChannelData(channel: number): Float32Array
}

export interface ScheduledSound {
  stop(): void
}

export interface AudioOutput {
  readonly running: boolean
  readonly currentTime: number
  createBuffer(channels: number, frames: number, sampleRate: number): PcmBuffer
  schedule(buffer: PcmBuffer, when: number, onEnded: () => void): ScheduledSound
}

export interface AudioPlayback {
  readonly state: Store<AudioState>
  play(output: AudioOutput | null): void
}

// Misc.audio_format comes first, then Opus in AudioFrame (rustdesk 1.5.1 audio_service.rs:614-708).
export class AudioPipeline implements AudioPlayback {
  readonly state = new ValueStore<AudioState>('ok')
  readonly #decoders: AudioDecoders
  readonly #now: () => number
  #format: AudioFormat | null = null
  #output: AudioOutput | null = null
  #decoder: OpusDecoder | null = null
  #failures = 0
  #failed = false
  #unsupported = false
  #opens: number[] = []
  #timestamp = 0
  #nextTime = 0
  readonly #queued = new Set<ScheduledSound>()
  #closed = false

  constructor(decoders: AudioDecoders, now: () => number = () => performance.now()) {
    this.#decoders = decoders
    this.#now = now
  }

  receive(message: Message): boolean {
    const { union } = message
    if (union.case === 'audioFrame') {
      this.#frame(union.value.data)
      return true
    }
    if (union.case === 'misc' && union.value.union.case === 'audioFormat') {
      this.#setFormat(union.value.union.value)
      return true
    }
    return false
  }

  play(output: AudioOutput | null): void {
    if (output === this.#output) return
    this.#stop()
    this.#output = this.#closed ? null : output
    if (!this.#output) return
    this.#failures = 0
    this.#failed = false
    this.#publish()
  }

  close(): void {
    this.#closed = true
    this.play(null)
  }

  #setFormat(told: AudioFormat): void {
    const format = audioFormat({ sampleRate: told.sampleRate, channels: told.channels })
    const same =
      format?.sampleRate === this.#format?.sampleRate && format?.channels === this.#format?.channels
    if (format && same) return
    this.#format = format
    this.#unsupported = false
    this.#stop()
    this.#publish()
    if (format) void this.#check(format)
  }

  async #check(format: AudioFormat): Promise<void> {
    let supported: boolean
    try {
      supported = await this.#decoders.supports(format)
    } catch {
      supported = false
    }
    if (supported || format !== this.#format) return
    this.#unsupported = true
    this.#stop()
    this.#publish()
  }

  #frame(data: Uint8Array): void {
    if (!this.#output?.running || !this.#format || this.state.get() !== 'ok') return
    if (!acceptsFrame(data)) return
    if (this.#decoder && this.#decoder.decodeQueueSize >= MAX_DECODE_QUEUE) return
    if (!this.#decoder && !this.#mayOpen()) {
      this.#giveUp()
      return
    }
    try {
      this.#decoder ??= this.#openDecoder(this.#format)
      this.#timestamp += PACKET_US
      this.#decoder.decode(data, this.#timestamp)
    } catch {
      this.#decoderFailed()
    }
  }

  #mayOpen(): boolean {
    const now = this.#now()
    this.#opens = this.#opens.filter((at) => now - at < REBUILD_WINDOW_MS)
    if (this.#opens.length > MAX_DECODER_REBUILDS) return false
    this.#opens.push(now)
    return true
  }

  #openDecoder(format: AudioFormat): OpusDecoder {
    const decoder: OpusDecoder = this.#decoders.create(format, {
      output: (data) => {
        if (decoder === this.#decoder) this.#played(data)
        else data.close()
      },
      error: () => {
        if (decoder === this.#decoder) this.#decoderFailed()
      },
    })
    return decoder
  }

  #played(data: DecodedAudio): void {
    try {
      this.#schedule(data)
      this.#failures = 0
    } catch {
      this.#countFailure()
    } finally {
      data.close()
    }
  }

  #schedule(data: DecodedAudio): void {
    const output = this.#output
    if (!output || data.numberOfFrames === 0) return
    const buffer = output.createBuffer(data.numberOfChannels, data.numberOfFrames, data.sampleRate)
    for (let planeIndex = 0; planeIndex < data.numberOfChannels; planeIndex++) {
      data.copyTo(buffer.getChannelData(planeIndex), { planeIndex, format: 'f32-planar' })
    }
    const now = output.currentTime
    if (this.#nextTime - now > MAX_BACKLOG_S) this.#flush()
    const at = this.#nextTime < now ? now + START_LEAD_S : this.#nextTime
    const sound = output.schedule(buffer, at, () => {
      this.#queued.delete(sound)
    })
    this.#queued.add(sound)
    this.#nextTime = at + buffer.duration
  }

  #decoderFailed(): void {
    this.#decoder?.close()
    this.#decoder = null
    this.#countFailure()
  }

  #countFailure(): void {
    this.#failures++
    if (this.#failures >= MAX_DECODE_FAILURES) this.#giveUp()
  }

  #giveUp(): void {
    this.#stop()
    this.#failed = true
    this.#publish()
  }

  #publish(): void {
    this.state.set(this.#unsupported ? 'unsupported' : this.#failed ? 'failed' : 'ok')
  }

  #flush(): void {
    for (const sound of [...this.#queued]) sound.stop()
    this.#queued.clear()
    this.#nextTime = 0
  }

  #stop(): void {
    this.#decoder?.close()
    this.#decoder = null
    this.#flush()
  }
}
