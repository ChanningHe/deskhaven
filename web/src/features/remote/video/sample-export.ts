import { CLIENT_VERSION, type VideoCodec } from '../handshake/login'
import type { EncodedVideoFrame, VideoFrame as VideoFrameMessage } from '../proto/gen/message_pb.js'
import { ValueStore, type Store } from '../session/store'

export const SAMPLE_VERSION = 1
export const SAMPLE_FRAMES = 30

/** The VideoFrame union case of each codec, whether we decode it or not. */
export type SampleCodec = `${VideoCodec}s`

export function sampleCodecOf(codec: VideoCodec): SampleCodec {
  return `${codec}s`
}

/** Their frames are worth keeping even short of SAMPLE_FRAMES: the session may give them up. */
const EXPERIMENTAL: readonly SampleCodec[] = ['h264s', 'h265s']

/** Long enough for a browser that starts the download late (FileSaver.js waits 40 s). */
export const REVOKE_AFTER_MS = 40_000

export interface SampleFrame {
  key: boolean
  pts: number
  /** Standard base64 of EncodedVideoFrame.data, as it came. */
  data: string
}

export interface SampleSource {
  peer: { platform: string; version: string }
  display: { width: number; height: number }
}

export interface VideoSample extends SampleSource {
  version: number
  clientVersion: string
  codec: SampleCodec
  /** False when the session moved off the codec before SAMPLE_FRAMES frames. */
  complete: boolean
  frames: SampleFrame[]
}

/** `frames` stays 0 until the key frame the recording starts at. */
export type SampleProgress = { kind: 'idle' } | { kind: 'recording'; frames: number }

const IDLE: SampleProgress = { kind: 'idle' }

interface Recording {
  resolve(sample: VideoSample | null): void
  codec: SampleCodec | null
  source: SampleSource | null
  frames: SampleFrame[]
}

function encoded(
  message: VideoFrameMessage,
): { codec: SampleCodec; frames: readonly EncodedVideoFrame[] } | null {
  const { union } = message
  switch (union.case) {
    case 'vp9s':
    case 'h264s':
    case 'h265s':
    case 'vp8s':
    case 'av1s':
      return { codec: union.case, frames: union.value.frames }
    default:
      return null
  }
}

/**
 * Records the peer's encoded frames of the codec the session asks for, from its key frame on, for
 * a test fixture. Frames of other codecs (in flight, or what the peer sends instead) are passed
 * over.
 */
export class SampleRecorder {
  readonly #progress = new ValueStore<SampleProgress>(IDLE)
  readonly #source: () => SampleSource
  readonly #requested: () => SampleCodec
  #recording: Recording | null = null

  constructor(source: () => SampleSource, requested: () => SampleCodec) {
    this.#source = source
    this.#requested = requested
  }

  get progress(): Store<SampleProgress> {
    return this.#progress
  }

  /** Resolves with SAMPLE_FRAMES frames, or null when cancelled or started again. */
  start(): Promise<VideoSample | null> {
    this.cancel()
    return new Promise((resolve) => {
      this.#recording = { resolve, codec: null, source: null, frames: [] }
      this.#progress.set({ kind: 'recording', frames: 0 })
    })
  }

  cancel(): void {
    this.#finish(null)
  }

  /**
   * The session asks for another codec. H264 or H265 frames recorded so far are given back, as
   * the session falls back from a codec it cannot decode; other codecs start over on the new one.
   */
  retarget(): void {
    const recording = this.#recording
    const locked = recording?.codec
    if (!recording || !locked || locked === this.#requested()) return
    if (EXPERIMENTAL.includes(locked)) {
      this.#finish(recording)
      return
    }
    recording.codec = null
    recording.source = null
    recording.frames = []
    this.#progress.set({ kind: 'recording', frames: 0 })
  }

  push(message: VideoFrameMessage): void {
    this.retarget()
    const recording = this.#recording
    const sample = recording && encoded(message)
    if (!recording || !sample) return
    const { codec } = sample
    if (codec !== (recording.codec ?? this.#requested())) return
    for (const frame of sample.frames) {
      if (recording.codec === null) {
        if (!frame.key) continue
        recording.codec = codec
        recording.source = this.#source()
      }
      recording.frames.push({
        key: frame.key,
        pts: Number(frame.pts),
        data: encodeBase64(frame.data),
      })
      if (recording.frames.length === SAMPLE_FRAMES) {
        this.#finish(recording)
        return
      }
    }
    const frames = recording.frames.length
    const progress = this.#progress.get()
    if (progress.kind !== 'recording' || progress.frames !== frames) {
      this.#progress.set({ kind: 'recording', frames })
    }
  }

  #finish(recording: Recording | null): void {
    const current = this.#recording
    if (!current) return
    this.#recording = null
    this.#progress.set(IDLE)
    const { codec, source } = current
    if (recording !== current || codec === null || source === null) {
      current.resolve(null)
      return
    }
    current.resolve({
      version: SAMPLE_VERSION,
      clientVersion: CLIENT_VERSION,
      codec,
      complete: current.frames.length === SAMPLE_FRAMES,
      peer: source.peer,
      display: source.display,
      frames: current.frames,
    })
  }
}

const BASE64_CHUNK = 0x8000

export function encodeBase64(bytes: Uint8Array): string {
  let binary = ''
  for (let i = 0; i < bytes.length; i += BASE64_CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(i, i + BASE64_CHUNK))
  }
  return btoa(binary)
}

const pad = (n: number) => String(n).padStart(2, '0')

/** `video-sample-<codec>-<yyyymmdd>-<hhmmss>.json`, in local time. */
export function sampleFileName(codec: SampleCodec, at: Date): string {
  const day = `${String(at.getFullYear())}${pad(at.getMonth() + 1)}${pad(at.getDate())}`
  const time = `${pad(at.getHours())}${pad(at.getMinutes())}${pad(at.getSeconds())}`
  return `video-sample-${codec}-${day}-${time}.json`
}

/** Hands the sample to the browser as a file download; nothing leaves this machine. */
export function downloadSample(sample: VideoSample, at = new Date()): void {
  const blob = new Blob([JSON.stringify(sample)], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = sampleFileName(sample.codec, at)
  anchor.click()
  setTimeout(() => {
    URL.revokeObjectURL(url)
  }, REVOKE_AFTER_MS)
}
