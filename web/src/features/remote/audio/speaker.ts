import { ValueStore } from '../session/store'
import { webCodecsDecodesOpus } from './decoder'
import type { AudioOutput, AudioState } from './pipeline'

export type SpeakerContext = Pick<
  AudioContext,
  | 'state'
  | 'currentTime'
  | 'destination'
  | 'createBuffer'
  | 'createBufferSource'
  | 'resume'
  | 'close'
  | 'onstatechange'
>

export interface AudioDeps {
  decodesOpus(): Promise<boolean>
  createContext(): SpeakerContext
}

export const browserAudio: AudioDeps = {
  decodesOpus: webCodecsDecodesOpus,
  createContext: () => new AudioContext({ latencyHint: 'interactive' }),
}

export interface SpeakerView {
  supported: boolean | null
  output: AudioOutput | null
  running: boolean
}

function outputOf(context: SpeakerContext): AudioOutput {
  return {
    get running() {
      return context.state === 'running'
    },
    get currentTime() {
      return context.currentTime
    },
    createBuffer: (channels, frames, sampleRate) =>
      context.createBuffer(channels, frames, sampleRate),
    schedule(buffer: AudioBuffer, when, onEnded) {
      const source = context.createBufferSource()
      source.buffer = buffer
      source.connect(context.destination)
      source.onended = onEnded
      source.start(when)
      return {
        stop: () => {
          try {
            source.stop()
          } catch {
            // Sound must never take the video session down with it.
          }
        },
      }
    },
  }
}

// One AudioContext per page, not per session: a reconnect has no click to resume a new one.
export class Speaker {
  readonly view = new ValueStore<SpeakerView>({ supported: null, output: null, running: false })
  readonly #deps: AudioDeps | undefined
  #probe: Promise<void> | null = null
  #context: SpeakerContext | null = null

  constructor(deps: AudioDeps | undefined) {
    this.#deps = deps
  }

  probe(): Promise<void> {
    this.#probe ??= this.#decodesOpus().then((supported) => {
      this.#update({ supported })
    })
    return this.#probe
  }

  prepare(): void {
    if (!this.#context) this.#open()
  }

  // Call it within the click: browsers resume a context only then.
  start(): void {
    const context = this.#context ?? this.#open()
    context?.resume().catch(() => undefined)
  }

  stop(): void {
    const context = this.#context
    if (!context) return
    this.#context = null
    context.onstatechange = null
    context.close().catch(() => undefined)
    this.#update({ output: null, running: false })
  }

  async #decodesOpus(): Promise<boolean> {
    try {
      return (await this.#deps?.decodesOpus()) ?? false
    } catch {
      return false
    }
  }

  #open(): SpeakerContext | null {
    if (!this.#deps || this.view.get().supported !== true) return null
    let context: SpeakerContext
    try {
      context = this.#deps.createContext()
    } catch {
      return null
    }
    this.#context = context
    context.onstatechange = () => {
      this.#update({ running: context.state === 'running' })
    }
    this.#update({ output: outputOf(context), running: context.state === 'running' })
    return context
  }

  #update(patch: Partial<SpeakerView>): void {
    this.view.set({ ...this.view.get(), ...patch })
  }
}

export type SoundSwitch =
  'probing' | 'unsupported' | 'denied' | 'off' | 'suspended' | 'on' | 'failed'

export function soundSwitch(
  speaker: SpeakerView,
  session: { enabled: boolean; permitted: boolean; audio: AudioState },
): SoundSwitch {
  if (speaker.supported === null) return 'probing'
  if (!speaker.supported || session.audio === 'unsupported') return 'unsupported'
  if (!session.permitted) return 'denied'
  if (!session.enabled) return 'off'
  if (session.audio === 'failed') return 'failed'
  return speaker.running ? 'on' : 'suspended'
}
