import type { VideoMeter } from '../video/pipeline'
import { ValueStore, type Store } from './store'

export interface SessionStats {
  fps: number
  kbps: number
  delayMs: number | null
  decodeQueue: number
  decodeMs: number | null
  keyRequests: number
  pointerMoves: number
  width: number
  height: number
  codec: string | null
}

export interface StatsProbe {
  decodeQueue: number
  codec: string | null
  width: number
  height: number
}

export interface StatsClock {
  now(): number
  every(ms: number, onTick: () => void): () => void
}

export const STATS_INTERVAL_MS = 1000

export const realClock: StatsClock = {
  now: () => performance.now(),
  every: (ms, onTick) => {
    const id = setInterval(onTick, ms)
    return () => {
      clearInterval(id)
    }
  },
}

export const NO_STATS: SessionStats = {
  fps: 0,
  kbps: 0,
  delayMs: null,
  decodeQueue: 0,
  decodeMs: null,
  keyRequests: 0,
  pointerMoves: 0,
  width: 0,
  height: 0,
  codec: null,
}

function oneDecimal(n: number): number {
  return Math.round(n * 10) / 10
}

export class SessionMeter implements VideoMeter {
  readonly #stats = new ValueStore<SessionStats>(NO_STATS)
  readonly #clock: StatsClock
  readonly #probe: () => StatsProbe
  #stopTimer: (() => void) | null = null
  #closed = false
  #since = 0
  #frames = 0
  #bytes = 0
  #decodeTotalMs = 0
  #moves = 0
  #keyRequests = 0
  #delayMs: number | null = null

  constructor(state: Store<{ kind: string }>, clock: StatsClock, probe: () => StatsProbe) {
    this.#clock = clock
    this.#probe = probe
    const unsubscribe = state.subscribe(() => {
      const { kind } = state.get()
      if (kind === 'active') this.#start()
      else if (kind === 'closed') {
        unsubscribe()
        this.#stop()
      }
    })
  }

  get stats(): Store<SessionStats> {
    return this.#stats
  }

  now(): number {
    return this.#clock.now()
  }

  received(bytes: number): void {
    this.#bytes += bytes
  }

  decoded(decodeMs: number): void {
    this.#frames++
    this.#decodeTotalMs += decodeMs
  }

  keyRequested(): void {
    this.#keyRequests++
  }

  pointerMoved(): void {
    this.#moves++
  }

  delay(ms: number): void {
    this.#delayMs = ms
  }

  #start(): void {
    if (this.#stopTimer || this.#closed) return
    this.#since = this.#clock.now()
    this.#stopTimer = this.#clock.every(STATS_INTERVAL_MS, () => {
      this.#tick()
    })
  }

  #stop(): void {
    this.#closed = true
    this.#stopTimer?.()
    this.#stopTimer = null
  }

  #tick(): void {
    const now = this.#clock.now()
    const elapsed = now - this.#since
    if (elapsed <= 0) return
    const perSecond = (n: number) => oneDecimal((n * 1000) / elapsed)
    const probe = this.#probe()
    this.#stats.set({
      fps: perSecond(this.#frames),
      kbps: Math.round((this.#bytes * 8) / elapsed),
      delayMs: this.#delayMs,
      decodeQueue: probe.decodeQueue,
      decodeMs: this.#frames === 0 ? null : oneDecimal(this.#decodeTotalMs / this.#frames),
      keyRequests: this.#keyRequests,
      pointerMoves: perSecond(this.#moves),
      width: probe.width,
      height: probe.height,
      codec: probe.codec,
    })
    this.#since = now
    this.#frames = 0
    this.#bytes = 0
    this.#decodeTotalMs = 0
    this.#moves = 0
  }
}
